#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dependency_impact_server::comparisons::serve().await
}
