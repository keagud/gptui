use gpt::app;
use gpt::cli::run_cli;
use gpt::session::{Role, Session};


async fn _main() -> anyhow::Result<()> {
    app::app_test().await?;

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run_cli().await
}
