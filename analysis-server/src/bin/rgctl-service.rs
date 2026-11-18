#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dependency_impact_server::service::serve().await
}
