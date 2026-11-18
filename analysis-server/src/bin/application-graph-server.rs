#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dependency_impact_server::onboarding::serve().await
}
