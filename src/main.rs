use std::io::{self, Read, Write};

use crossterm::QueueableCommand;
use ctrlc::set_handler;
use gpt::app;
use gpt::cli::run_cli;
use gpt::session::{stream_user_message, Session, Thread};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut session = Session::new_dummy()?;
    let thread_id = session.new_thread("You are a helpful assistant")?;
    let thread = session.thread_by_id(thread_id).unwrap();

    let mut rx = stream_user_message(
        "What is the southernmost national capital in the world?",
        &thread,
    )?;

    let mut stdout = std::io::stdout();

    while let Some(reply) = rx.recv().await {
        stdout.write_all(reply.as_bytes())?;
        stdout.flush()?;
    }

    Ok(())
}
