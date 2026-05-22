#[tokio::main]
async fn main() -> anyhow::Result<()> {
    sorotte_cli::run_sorotte_cli_from_env().await
}
