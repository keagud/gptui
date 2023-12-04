#![allow(dead_code)]

use std::io::{self, Write};

use gpt::app;
use gpt::{tui, Role, Session};

use futures::{stream, Stream, StreamExt};
use futures_util::pin_mut;

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

async fn _main() -> anyhow::Result<()> {
    app::app_test().await?;

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut session = Session::new_stdout()?;
    session.load_threads()?;

    for thread in session.threads.values() {
        let first_user_msg = thread
            .messages
            .iter()
            .filter(|m| m.role == Role::User)
            .min_by_key(|m| m.timestamp.floor() as usize);

        if let Some(msg) = first_user_msg {
            println!("{} | {}", msg.timestamp, msg.content);
        }
    }

    let thread_id = session.new_thread("You are a helpful assistant")?;

    let mut stream = session
        .stream_user_message(
            "Write a poem about the Rust programming langauge in the style of Edgar Allen Poe",
            thread_id,
        )
        .await?;

    pin_mut!(stream);

    while let Some(s) = stream.next().await.flatten() {
        print!("{:?}", s);
        io::stdout().flush()?;
    }

    session.save_to_db()?;

    Ok(())
}
