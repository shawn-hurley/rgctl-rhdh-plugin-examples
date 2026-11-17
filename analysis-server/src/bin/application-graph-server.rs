#[path = "../onboarding/mod.rs"]
mod onboarding;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    onboarding::serve().await
}
