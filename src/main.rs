use std::io::{self, Read, Write};

use crossterm::QueueableCommand;
use ctrlc::set_handler;
use gpt::app;
use gpt::cli::run_cli;


#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        crossterm::terminal::disable_raw_mode().unwrap();
    });

    run_cli().await
}
