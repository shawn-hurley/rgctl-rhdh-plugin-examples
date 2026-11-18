use crate::{analysis, model};
use axum::{
    Json, Router,
    extract::{Path, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use model::{Comparison, Config, Job, Problem, now};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    queue: mpsc::Sender<String>,
    token: Arc<String>,
}
struct ApiError(StatusCode, &'static str, String);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (
            self.0,
            Json(json!({"error":{"code":self.1,"message":self.2}})),
        )
            .into_response();
        if self.0 == StatusCode::SERVICE_UNAVAILABLE {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, "5".parse().expect("valid header"));
        }
        response
    }
}
fn failure(error: impl std::fmt::Display) -> ApiError {
    eprintln!("Storage error: {error}");
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "STORAGE_ERROR",
        "Job storage is unavailable".into(),
    )
}
fn job_path(config: &Config, id: &str) -> PathBuf {
    config.data_dir.join(id)
}
fn save_job(config: &Config, job: &Job) -> anyhow::Result<()> {
    let dir = job_path(config, &job.id);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("job.json.tmp"), serde_json::to_vec_pretty(job)?)?;
    std::fs::rename(dir.join("job.json.tmp"), dir.join("job.json"))?;
    Ok(())
}
async fn read_job(config: &Config, id: &str) -> Result<Job, ApiError> {
    if Uuid::parse_str(id).is_err() {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "Comparison not found".into(),
        ));
    }
    let bytes = tokio::fs::read(job_path(config, id).join("job.json"))
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ApiError(
                    StatusCode::NOT_FOUND,
                    "NOT_FOUND",
                    "Comparison not found".into(),
                )
            } else {
                failure(e)
            }
        })?;
    serde_json::from_slice(&bytes).map_err(failure)
}
async fn authenticate(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let valid = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|s| s.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .is_some_and(|s| s == state.token.as_str());
    if !valid {
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
    input: Result<Json<Comparison>, axum::extract::rejection::JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let Json(request) =
        input.map_err(|e| ApiError(StatusCode::BAD_REQUEST, "INVALID_REQUEST", e.body_text()))?;
    request
        .validate()
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, "INVALID_REQUEST", e.to_string()))?;
    let application = state
        .config
        .applications
        .get(&request.entity_ref)
        .ok_or_else(|| {
            ApiError(
                StatusCode::BAD_REQUEST,
                "UNKNOWN_APPLICATION",
                "Application is not configured for analysis".into(),
            )
        })?;
    if request.application_revision != application.revision {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "UNSUPPORTED_REVISION",
            "Requested revision is not configured for this application".into(),
        ));
    }
    let permit = state.queue.try_reserve().map_err(|_| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "QUEUE_FULL",
            "Analysis queue is full; retry later".into(),
        )
    })?;
    let job = Job {
        id: Uuid::new_v4().to_string(),
        request,
        state: "queued".into(),
        stage: None,
        created_at: now(),
        updated_at: now(),
        error: None,
    };
    save_job(&state.config, &job).map_err(failure)?;
    permit.send(job.id.clone());
    Ok((StatusCode::ACCEPTED, Json(job)))
}
async fn status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Job>, ApiError> {
    Ok(Json(read_job(&state.config, &id).await?))
}
async fn report(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let job = read_job(&state.config, &id).await?;
    if job.state != "succeeded" {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "REPORT_NOT_READY",
            format!("Comparison is {}", job.state),
        ));
    }
    let bytes = tokio::fs::read(job_path(&state.config, &id).join("report.json"))
        .await
        .map_err(failure)?;
    Ok(Json(serde_json::from_slice(&bytes).map_err(failure)?))
}
fn execute_job(config: &Config, id: &str) -> anyhow::Result<()> {
    let mut job: Job =
        serde_json::from_slice(&std::fs::read(job_path(config, id).join("job.json"))?)?;
    job.state = "running".into();
    job.updated_at = now();
    save_job(config, &job)?;
    let result = analysis::compare(config, &job.request, &job_path(config, id), &|stage| {
        let mut update = job.clone();
        update.stage = Some(stage.into());
        update.updated_at = now();
        save_job(config, &update)
    });
    match result {
        Ok(report) => {
            let dir = job_path(config, id);
            std::fs::write(
                dir.join("report.json.tmp"),
                serde_json::to_vec_pretty(&report)?,
            )?;
            std::fs::rename(dir.join("report.json.tmp"), dir.join("report.json"))?;
            job.state = "succeeded".into();
        }
        Err(error) => {
            eprintln!("Comparison {id} failed: {error:#}");
            job.state = "failed".into();
            job.error = Some(Problem {
                code: "ANALYSIS_FAILED".into(),
                message: format!(
                    "Analysis failed: {}. See service logs for details.",
                    error
                        .to_string()
                        .split(';')
                        .next()
                        .unwrap_or("unknown error")
                ),
            });
        }
    }
    job.stage = None;
    job.updated_at = now();
    save_job(config, &job)
}

pub async fn router(
    mut config: Config,
    token: String,
    limiter: Arc<tokio::sync::Semaphore>,
) -> anyhow::Result<Router> {
    anyhow::ensure!(
        config.queue_capacity > 0 && config.queue_capacity <= 100,
        "Queue capacity must be between 1 and 100"
    );
    std::fs::create_dir_all(&config.data_dir)?;
    config.data_dir = std::fs::canonicalize(&config.data_dir)?;
    for application in config.applications.values_mut() {
        application.path = std::fs::canonicalize(&application.path)?;
    }
    if let Some(repo) = &mut config.maven_repository {
        std::fs::create_dir_all(&repo)?;
        *repo = std::fs::canonicalize(&repo)?;
    }
    anyhow::ensure!(
        token.len() >= 16,
        "IMPACT_SERVICE_TOKEN must contain at least 16 characters"
    );
    // Single service instance per data volume. Interrupted work is explicit;
    // persisted source/report files are never presented as a completed job.
    for entry in std::fs::read_dir(&config.data_dir)? {
        let entry = entry?;
        let path = entry.path().join("job.json");
        if !path.is_file() {
            continue;
        }
        let mut job: Job = serde_json::from_slice(&std::fs::read(path)?)?;
        if job.state == "queued" || job.state == "running" {
            job.state = "failed".into();
            job.stage = None;
            job.updated_at = now();
            job.error = Some(Problem {
                code: "INTERRUPTED".into(),
                message: "Service restarted before this comparison completed; submit again".into(),
            });
            save_job(&config, &job)?;
        }
    }
    let (queue, mut receiver) = mpsc::channel::<String>(config.queue_capacity);
    let config = Arc::new(config);
    let worker_config = config.clone();
    tokio::spawn(async move {
        while let Some(id) = receiver.recv().await {
            let Ok(_permit) = limiter.clone().acquire_owned().await else {
                break;
            };
            let config = worker_config.clone();
            let failed_id = id.clone();
            let result = tokio::task::spawn_blocking(move || execute_job(&config, &id)).await;
            if !matches!(result, Ok(Ok(()))) {
                eprintln!("Job {failed_id} worker error: {result:?}");
                if let Ok(bytes) =
                    std::fs::read(job_path(&worker_config, &failed_id).join("job.json"))
                {
                    if let Ok(mut job) = serde_json::from_slice::<Job>(&bytes) {
                        job.state = "failed".into();
                        job.stage = None;
                        job.updated_at = now();
                        job.error = Some(Problem {
                            code: "WORKER_FAILED".into(),
                            message: "Analysis worker failed; see service logs".into(),
                        });
                        if let Err(e) = save_job(&worker_config, &job) {
                            eprintln!("Cannot save failed job: {e}");
                        }
                    }
                }
            }
        }
    });
    let state = AppState {
        config,
        queue,
        token: Arc::new(token),
    };
    let comparisons = Router::new()
        .route("/v1/comparisons", post(submit))
        .route("/v1/comparisons/{id}", get(status))
        .route("/v1/comparisons/{id}/report", get(report))
        .route_layer(middleware::from_fn_with_state(state.clone(), authenticate));
    Ok(comparisons.with_state(state))
}

/// Compatibility entry point; normal deployment uses the combined service.
pub async fn serve() -> anyhow::Result<()> {
    let path = std::env::var("IMPACT_CONFIG").unwrap_or_else(|_| "config.json".into());
    let config: Config = serde_json::from_slice(&std::fs::read(path)?)?;
    let token = std::env::var("IMPACT_SERVICE_TOKEN")?;
    let app = router(config, token, Arc::new(tokio::sync::Semaphore::new(1)))
        .await?
        .route("/health", get(|| async { Json(json!({"status":"ready"})) }));
    crate::service::listen(
        app,
        std::env::var("IMPACT_LISTEN").unwrap_or_else(|_| "127.0.0.1:7070".into()),
    )
    .await
}
