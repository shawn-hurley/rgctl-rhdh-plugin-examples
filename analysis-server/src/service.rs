//! A single authenticated HTTP server and worker budget for both plugins.
use crate::{comparisons, model, onboarding};
use anyhow::{Result, ensure};
use axum::{Json, Router, routing::get};
use serde::Deserialize;
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Semaphore;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub comparisons: model::Config,
    pub application_graphs: onboarding::Config,
    #[serde(default = "concurrency")]
    pub max_concurrent_jobs: usize,
}
fn concurrency() -> usize {
    1
}

pub async fn router(config: Config, token: String) -> Result<Router> {
    Ok(build_router(config, token).await?.0)
}
async fn build_router(mut config: Config, token: String) -> Result<(Router, Arc<Semaphore>)> {
    ensure!(
        token.len() >= 16,
        "RGCTL_SERVICE_TOKEN must contain at least 16 characters"
    );
    ensure!(
        (1..=16).contains(&config.max_concurrent_jobs),
        "maxConcurrentJobs must be between 1 and 16"
    );
    for dir in [
        &config.comparisons.data_dir,
        &config.application_graphs.data_dir,
    ] {
        std::fs::create_dir_all(dir)?;
    }
    config.comparisons.data_dir = config.comparisons.data_dir.canonicalize()?;
    config.application_graphs.data_dir = config.application_graphs.data_dir.canonicalize()?;
    ensure!(
        !config
            .comparisons
            .data_dir
            .starts_with(&config.application_graphs.data_dir)
            && !config
                .application_graphs
                .data_dir
                .starts_with(&config.comparisons.data_dir),
        "Comparison and graph data directories must be separate, non-nested stores"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let a = std::fs::metadata(&config.comparisons.data_dir)?;
        let b = std::fs::metadata(&config.application_graphs.data_dir)?;
        ensure!(
            (a.dev(), a.ino()) != (b.dev(), b.ino()),
            "Comparison and graph stores must not alias the same directory"
        );
    }
    let limiter = Arc::new(Semaphore::new(config.max_concurrent_jobs));
    let comparisons =
        comparisons::router(config.comparisons, token.clone(), limiter.clone()).await?;
    let graphs = onboarding::router(config.application_graphs, token, limiter.clone()).await?;
    let app = Router::new()
        .route(
            "/health",
            get(|| async {
                Json(json!({"status":"ready","features":["dependency-impact","application-graph"]}))
            }),
        )
        .merge(comparisons)
        .merge(graphs);
    Ok((app, limiter))
}

pub async fn serve() -> Result<()> {
    let path =
        std::env::var("RGCTL_SERVICE_CONFIG").unwrap_or_else(|_| "service-config.json".into());
    let config: Config = serde_json::from_slice(&std::fs::read(PathBuf::from(path))?)?;
    let token = std::env::var("RGCTL_SERVICE_TOKEN")?;
    let app = router(config, token).await?;
    listen(
        app,
        std::env::var("RGCTL_SERVICE_LISTEN").unwrap_or_else(|_| "127.0.0.1:7070".into()),
    )
    .await
}
pub(crate) async fn listen(app: Router, address: String) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(&address).await?;
    println!("rgctl service listening on {address}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

async fn shutdown() {
    #[cfg(unix)]
    {
        if let Ok(mut terminate) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! {_ = tokio::signal::ctrl_c()=>{},_ = terminate.recv()=>{}}
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use serde_json::Value;
    use tower::ServiceExt;
    const TOKEN: &str = "shared-service-test-token";
    fn config(root: &std::path::Path) -> Config {
        serde_json::from_value(json!({
            "comparisons":{"applications":{},"dataDir":root.join("comparisons"),"queueCapacity":8},
            "applicationGraphs":{"dataDir":root.join("graphs"),"allowedRepositoryHosts":["github.com"],"allowedSourceRoots":[root],"queueCapacity":8}
        })).unwrap()
    }
    async fn call(
        app: &Router,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("Content-Type", "application/json");
        if let Some(token) = token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        let response = app
            .clone()
            .oneshot(
                req.body(
                    body.map(|v| Body::from(v.to_string()))
                        .unwrap_or_else(Body::empty),
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
    #[tokio::test]
    async fn one_router_serves_and_authenticates_both_workflows() {
        let temp = tempfile::tempdir().unwrap();
        let app = router(config(temp.path()), TOKEN.into()).await.unwrap();
        let (status, health) = call(&app, "GET", "/health", None, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            health["features"],
            json!(["dependency-impact", "application-graph"])
        );
        for path in [
            "/v1/applications",
            "/v1/comparisons/11111111-1111-1111-1111-111111111111",
        ] {
            for token in [None, Some("wrong-service-token")] {
                assert_eq!(
                    call(&app, "GET", path, token, None).await.0,
                    StatusCode::UNAUTHORIZED
                );
            }
        }
        assert_eq!(
            call(&app, "GET", "/v1/applications", Some(TOKEN), None)
                .await
                .1["applications"],
            json!([])
        );
        for path in ["/v1/comparisons", "/v1/onboard"] {
            assert_eq!(
                call(&app, "POST", path, Some(TOKEN), Some(json!({})))
                    .await
                    .0,
                StatusCode::BAD_REQUEST
            );
        }
    }
    #[tokio::test]
    async fn shared_router_indexes_a_git_application_and_keeps_job_stores_isolated() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repository");
        std::fs::create_dir_all(source.join("src/main/java")).unwrap();
        std::fs::write(
            source.join("src/main/java/App.java"),
            "public class App {public String run(){return \"ok\";}}",
        )
        .unwrap();
        for args in [
            vec!["init", "-b", "main"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=Graph Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "-m",
                "test source",
            ],
        ] {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(&source)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let input = temp.path().join("comparison-input");
        std::fs::create_dir_all(&input).unwrap();
        let mut cfg = config(temp.path());
        cfg.comparisons.applications.insert(
            "component:default/comparison".into(),
            model::Application {
                path: input.clone(),
                revision: "main".into(),
            },
        );
        let (app, budget) = build_router(cfg, TOKEN.into()).await.unwrap();
        let held = budget.acquire().await.unwrap();
        // Make the comparison fail before invoking Maven once its slot opens.
        std::fs::remove_dir(input).unwrap();
        let (accepted,comparison)=call(&app,"POST","/v1/comparisons",Some(TOKEN),Some(json!({"entityRef":"component:default/comparison","applicationRevision":"main","dependency":{"groupId":"g","artifactId":"a"},"targetVersion":"2","strategy":"direct-upgrade"}))).await;
        assert_eq!(accepted, StatusCode::ACCEPTED, "{comparison}");
        let comparison_id = comparison["id"].as_str().unwrap();
        let (status,job)=call(&app,"POST","/v1/onboard",Some(TOKEN),Some(json!({"entityRef":"component:default/test","source":{"repositoryUrl":url::Url::from_directory_path(&source).unwrap().to_string()}}))).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{job}");
        let id = job["id"].as_str().unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        assert_eq!(
            call(&app, "GET", &format!("/v1/jobs/{id}"), Some(TOKEN), None)
                .await
                .1["state"],
            "queued"
        );
        assert_eq!(
            call(
                &app,
                "GET",
                &format!("/v1/comparisons/{comparison_id}"),
                Some(TOKEN),
                None
            )
            .await
            .1["state"],
            "queued"
        );
        drop(held);
        let mut ready = false;
        for _ in 0..250 {
            let (_, job) = call(&app, "GET", &format!("/v1/jobs/{id}"), Some(TOKEN), None).await;
            assert_ne!(job["state"], "failed", "{job}");
            if job["state"] == "succeeded" {
                ready = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(ready, "Graph job timed out");
        let (_, apps) = call(&app, "GET", "/v1/applications", Some(TOKEN), None).await;
        let key = apps["applications"][0]["applicationKey"].as_str().unwrap();
        let (status, nodes) = call(
            &app,
            "GET",
            &format!("/v1/applications/{key}/nodes"),
            Some(TOKEN),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(nodes["total"].as_u64().unwrap() > 0);
        assert_eq!(
            call(
                &app,
                "GET",
                &format!("/v1/comparisons/{id}"),
                Some(TOKEN),
                None
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        assert!(
            temp.path()
                .join("graphs/versions")
                .join(id)
                .join("graph.snapshot.bin")
                .is_file()
        );
        assert!(!temp.path().join("comparisons").join(id).exists());
    }
    #[tokio::test]
    async fn restart_recovers_interrupted_jobs_from_both_stores() {
        let temp = tempfile::tempdir().unwrap();
        let id = "11111111-1111-1111-1111-111111111111";
        let comparison = temp.path().join("comparisons").join(id);
        let graph = temp.path().join("graphs/versions").join(id);
        std::fs::create_dir_all(&comparison).unwrap();
        std::fs::create_dir_all(&graph).unwrap();
        std::fs::write(comparison.join("job.json"),json!({"id":id,"request":{"entityRef":"component:default/test","applicationRevision":"main","dependency":{"groupId":"g","artifactId":"a"},"targetVersion":"2","strategy":"direct-upgrade"},"state":"running","stage":"index","createdAt":1,"updatedAt":1,"error":null}).to_string()).unwrap();
        std::fs::write(graph.join("job.json"),json!({"id":id,"request":{"entityRef":"component:default/test","source":{"repositoryUrl":"https://github.com/example/test.git"}},"state":"queued","createdAt":1,"updatedAt":1,"graphVersion":null,"error":null}).to_string()).unwrap();
        let app = router(config(temp.path()), TOKEN.into()).await.unwrap();
        for path in [format!("/v1/comparisons/{id}"), format!("/v1/jobs/{id}")] {
            let (status, job) = call(&app, "GET", &path, Some(TOKEN), None).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(job["state"], "failed");
            assert!(job["error"].to_string().contains("INTERRUPTED"));
        }
    }
    #[tokio::test]
    async fn shared_startup_rejects_unsafe_store_aliases_credentials_and_limits() {
        let temp = tempfile::tempdir().unwrap();
        let mut cfg = config(temp.path());
        cfg.application_graphs.data_dir = cfg.comparisons.data_dir.clone();
        assert!(router(cfg, TOKEN.into()).await.is_err());
        let mut cfg = config(temp.path());
        cfg.application_graphs.data_dir = cfg.comparisons.data_dir.join("graphs");
        assert!(router(cfg, TOKEN.into()).await.is_err());
        assert!(router(config(temp.path()), "short".into()).await.is_err());
        let mut cfg = config(temp.path());
        cfg.max_concurrent_jobs = 0;
        assert!(router(cfg, TOKEN.into()).await.is_err());
    }
}
