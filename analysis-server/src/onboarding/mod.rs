mod connections;
mod dependencies;
mod index;
mod model;
mod openapi;
mod store;
#[cfg(test)]
mod tests;
mod trace;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
pub use model::Config;
use model::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::mpsc;

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    queue: mpsc::Sender<String>,
    token: Arc<String>,
}
struct ApiError(StatusCode, &'static str, String);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(json!({"error":{"code":self.1,"message":self.2}})),
        )
            .into_response()
    }
}
fn failure(error: impl std::fmt::Display) -> ApiError {
    eprintln!("Application graph storage: {error}");
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "STORAGE_ERROR",
        "Application graph storage is unavailable".into(),
    )
}
fn not_found(error: impl std::fmt::Display) -> ApiError {
    eprintln!("Application graph lookup: {error}");
    ApiError(
        StatusCode::NOT_FOUND,
        "NOT_FOUND",
        "Application graph or job not found".into(),
    )
}
async fn authenticate(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        != Some(state.token.as_str())
    {
        return ApiError(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "Service credentials required".into(),
        )
        .into_response();
    }
    next.run(request).await
}
async fn submit(
    State(state): State<AppState>,
    input: Result<Json<Onboard>, axum::extract::rejection::JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let Json(request) =
        input.map_err(|e| ApiError(StatusCode::BAD_REQUEST, "INVALID_REQUEST", e.body_text()))?;
    store::validate(&state.config, &request)
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, "INVALID_REQUEST", e.to_string()))?;
    let permit = state.queue.try_reserve().map_err(|_| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "QUEUE_FULL",
            "Onboarding queue is full".into(),
        )
    })?;
    let job = Job {
        id: uuid::Uuid::new_v4().to_string(),
        request,
        state: "queued".into(),
        created_at: now(),
        updated_at: now(),
        graph_version: None,
        error: None,
    };
    store::write(
        &store::job_dir(&state.config, &job.id)
            .map_err(failure)?
            .join("job.json"),
        &job,
    )
    .map_err(failure)?;
    permit.send(job.id.clone());
    Ok((StatusCode::ACCEPTED, Json(job)))
}
async fn job(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<Job>, ApiError> {
    let path = store::job_dir(&state.config, &id)
        .map_err(not_found)?
        .join("job.json");
    Ok(Json(store::read(&path).map_err(not_found)?))
}
async fn applications(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"applications":store::list(&state.config).map_err(failure)?}),
    ))
}
async fn application(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<Manifest>, ApiError> {
    Ok(Json(
        store::current(&state.config, &key).map_err(not_found)?,
    ))
}
async fn connections(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let list = store::list(&state.config).map_err(failure)?;
    Ok(Json(json!({"connections":connections::connect(&list)})))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogApplication {
    entity_ref: String,
    provides_apis: Vec<String>,
    consumes_apis: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogLinks {
    applications: Vec<CatalogApplication>,
}
async fn live_connections(
    State(state): State<AppState>,
    Json(links): Json<CatalogLinks>,
) -> Result<Json<Value>, ApiError> {
    if links.applications.len() > 1000
        || links.applications.iter().any(|a| {
            !entity_ref(&a.entity_ref, "component")
                || a.provides_apis.len() > 100
                || a.consumes_apis.len() > 100
                || a.provides_apis
                    .iter()
                    .chain(&a.consumes_apis)
                    .any(|r| !entity_ref(r, "api"))
        })
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "INVALID_CATALOG_RELATIONS",
            "Invalid catalog API relationships".into(),
        ));
    }
    let stored = store::list(&state.config).map_err(failure)?;
    let mut current = vec![];
    for mut application in stored {
        if let Some(relations) = links
            .applications
            .iter()
            .find(|r| r.entity_ref == application.entity_ref)
        {
            application.provides_apis = relations.provides_apis.clone();
            application.consumes_apis = relations.consumes_apis.clone();
            current.push(application);
        }
    }
    Ok(Json(json!({"connections":connections::connect(&current)})))
}
async fn version(
    State(state): State<AppState>,
    Path((key, id)): Path<(String, String)>,
) -> Result<Json<Manifest>, ApiError> {
    Ok(Json(
        store::version(&state.config, &key, &id).map_err(not_found)?,
    ))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Page {
    #[serde(default)]
    offset: usize,
    #[serde(default = "limit")]
    limit: usize,
    node_id: Option<String>,
    graph_version: Option<String>,
}
fn limit() -> usize {
    100
}
async fn nodes(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(page): Query<Page>,
) -> Result<Json<Value>, ApiError> {
    if page.limit == 0 || page.limit > 1000 {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "INVALID_PAGE",
            "Limit must be between 1 and 1000".into(),
        ));
    }
    let manifest = match page.graph_version.as_deref() {
        Some(id) => store::version(&state.config, &key, id),
        None => store::current(&state.config, &key),
    }
    .map_err(not_found)?;
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
        let graph = index::snapshot(&config, &manifest)?;
        let mut result = vec![]; let mut count = 0;
        graph.for_each_node_seq(|node| {
            if page.node_id.as_ref().is_some_and(|id| *id != node.id.to_string()) { return }
            if count >= page.offset && result.len() < page.limit { result.push(json!(node)); }
            count += 1;
        })?;
        Ok(json!({"entityRef":manifest.entity_ref,"graphVersion":manifest.graph_version,"total":count,"offset":page.offset,"nodes":result}))
    }).await.map_err(failure)?.map_err(failure)?;
    Ok(Json(result))
}
async fn edges(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(page): Query<Page>,
) -> Result<Json<Value>, ApiError> {
    if page.limit == 0 || page.limit > 1000 {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "INVALID_PAGE",
            "Limit must be between 1 and 1000".into(),
        ));
    }
    let manifest = match page.graph_version.as_deref() {
        Some(id) => store::version(&state.config, &key, id),
        None => store::current(&state.config, &key),
    }
    .map_err(not_found)?;
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
        let graph = index::snapshot(&config, &manifest)?;
        let mut result = vec![]; let mut count = 0;
        graph.for_each_edge(|from,to,kind| {
            if page.node_id.as_ref().is_some_and(|id| *id != from.to_string() && *id != to.to_string()) { return Ok(()) }
            if count >= page.offset && result.len() < page.limit { result.push(json!({"from":from,"to":to,"type":kind})); }
            count += 1; Ok(())
        })?;
        Ok(json!({"entityRef":manifest.entity_ref,"graphVersion":manifest.graph_version,"total":count,"offset":page.offset,"edges":result}))
    }).await.map_err(failure)?.map_err(failure)?;
    Ok(Json(result))
}
fn execute(config: &Config, id: &str) -> anyhow::Result<()> {
    let dir = store::job_dir(config, id)?;
    let mut job: Job = store::read(&dir.join("job.json"))?;
    job.state = "running".into();
    job.updated_at = now();
    store::write(&dir.join("job.json"), &job)?;
    let result = (|| -> anyhow::Result<Manifest> {
        let (source, commit) = store::checkout(&job.request, &dir)?;
        index::index(
            &job.request,
            id,
            &dir,
            &source,
            commit,
            config.maven_repository.clone(),
        )
    })();
    match result {
        Ok(manifest) => {
            store::write(&dir.join("manifest.json"), &manifest)?;
            store::write(
                &config
                    .data_dir
                    .join("applications")
                    .join(format!("{}.json", manifest.application_key)),
                &manifest,
            )?;
            job.state = "succeeded".into();
            job.graph_version = Some(id.into());
        }
        Err(error) => {
            eprintln!("Onboarding {id}: {error:#}");
            job.state = "failed".into();
            job.error = Some(error.to_string());
        }
    }
    job.updated_at = now();
    store::write(&dir.join("job.json"), &job)
}

pub async fn router(
    mut config: Config,
    token: String,
    limiter: Arc<tokio::sync::Semaphore>,
) -> anyhow::Result<Router> {
    anyhow::ensure!(
        config.queue_capacity > 0 && config.queue_capacity <= 100,
        "Queue capacity must be 1–100"
    );
    std::fs::create_dir_all(config.data_dir.join("versions"))?;
    config.data_dir = config.data_dir.canonicalize()?;
    anyhow::ensure!(
        token.len() >= 16,
        "Service token must have at least 16 characters"
    );
    for entry in std::fs::read_dir(config.data_dir.join("versions"))? {
        let path = entry?.path().join("job.json");
        if !path.is_file() {
            continue;
        }
        let mut job: Job = store::read(&path)?;
        if job.state == "queued" || job.state == "running" {
            job.state = "failed".into();
            job.error = Some("INTERRUPTED: service restarted before onboarding completed".into());
            job.updated_at = now();
            store::write(&path, &job)?;
        }
    }
    let (queue, mut receive) = mpsc::channel::<String>(config.queue_capacity);
    let state = AppState {
        config: Arc::new(config),
        queue,
        token: Arc::new(token),
    };
    let worker = state.config.clone();
    tokio::spawn(async move {
        while let Some(id) = receive.recv().await {
            let Ok(_permit) = limiter.clone().acquire_owned().await else {
                break;
            };
            let config = worker.clone();
            let failed_id = id.clone();
            let result = tokio::task::spawn_blocking(move || execute(&config, &id)).await;
            if !matches!(result, Ok(Ok(()))) {
                eprintln!("Onboarding worker failed: {result:?}");
                if let Ok(dir) = store::job_dir(&worker, &failed_id) {
                    if let Ok(mut job) = store::read::<Job>(&dir.join("job.json")) {
                        job.state = "failed".into();
                        job.error = Some("WORKER_FAILED".into());
                        job.updated_at = now();
                        let _ = store::write(&dir.join("job.json"), &job);
                    }
                }
            }
        }
    });
    let routes = Router::new()
        .route("/v1/onboard", post(submit))
        .route("/v1/jobs/{id}", get(job))
        .route("/v1/applications", get(applications))
        .route("/v1/applications/{key}", get(application))
        .route("/v1/applications/{key}/versions/{id}", get(version))
        .route("/v1/applications/{key}/nodes", get(nodes))
        .route("/v1/applications/{key}/edges", get(edges))
        .route("/v1/connections", get(connections).post(live_connections))
        .layer(DefaultBodyLimit::max(256 * 1024))
        .route_layer(middleware::from_fn_with_state(state.clone(), authenticate));
    Ok(routes.with_state(state))
}

/// Compatibility entry point; normal deployment uses the combined service.
pub async fn serve() -> anyhow::Result<()> {
    let path = std::env::var("APPLICATION_GRAPH_CONFIG")
        .unwrap_or_else(|_| "application-graph.json".into());
    let config: Config = store::read(&PathBuf::from(path))?;
    let token = std::env::var("APPLICATION_GRAPH_SERVICE_TOKEN")?;
    let app = router(config, token, Arc::new(tokio::sync::Semaphore::new(1)))
        .await?
        .route("/health", get(|| async { Json(json!({"status":"ready"})) }));
    crate::service::listen(
        app,
        std::env::var("APPLICATION_GRAPH_LISTEN").unwrap_or_else(|_| "127.0.0.1:7071".into()),
    )
    .await
}
