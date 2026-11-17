#[allow(dead_code)]
#[path = "../dispatch.rs"]
mod dispatch;
#[allow(dead_code)]
#[path = "../maven.rs"]
mod maven;
#[allow(dead_code)]
#[path = "../model.rs"]
mod model;
#[path = "../onboarding/mod.rs"]
mod onboarding;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    onboarding::serve().await
}
