#![allow(dead_code)]

use std::io::{self, Write};
use tokio::io::AsyncBufReadExt;

const OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";
const MAX_TOKENS: usize = 200;

async fn run_shell() -> anyhow::Result<()> {
    let stdout = io::stdout();

    let mut session = gpt::Session::new(stdout)?;
    let thread_id = session.new_thread("You are a helpful assistant")?;

    let stdin = io::stdin();
    let mut buf = String::new();

    loop {
        print!(">> ");
        io::stdout().flush()?;

        stdin.read_line(&mut buf)?;

        if buf.is_empty() {
            continue;
        }

        if buf.to_lowercase().trim() == "q" {
            break;
        }

        session.send_user_message(&buf, thread_id).await?;
        println!();
        io::stdout().flush()?;
        buf.clear();
    }

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run_shell().await?;
    Ok(())
}
