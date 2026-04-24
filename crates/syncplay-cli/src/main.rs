#[tokio::main]
async fn main() -> anyhow::Result<()> {
    syncplay_cli::run_syncplay_cli_from_env().await
}
