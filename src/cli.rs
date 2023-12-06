use crate::session::{Message, Role, Session, Thread};
use anyhow;
use anyhow::format_err;
use chrono::Utc;
use colored::Colorize;
use futures_util::pin_mut;
use itertools::Itertools;
use std::io::{self, Read, Write};
use arboard;

use clap::{Parser, Subcommand};
use futures::{Stream, StreamExt};

const DEFAULT_PROMPT: &str = r#"You are a helpful assistant"#;
const INPUT_INDICATOR: &str = ">> ";

#[derive(Parser, Debug)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    List,
    New { prompt: Option<String> },
    Resume { index: i64 },
}

fn clear_screen() -> anyhow::Result<()> {
    if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/c", "cls"])
            .status()?;
    } else {
        std::process::Command::new("clear").status()?;
    }

    Ok(())
}

fn print_message(msg: &Message) -> Option<()> {
    match msg.role {
        Role::System => return None,
        Role::User => {
            println!("{}", "<User>".green().bold().underline());
            println!("{}{}", INPUT_INDICATOR, msg.content);
        }
        Role::Assistant => {
            println!("{}", "<Assistant>".blue().bold().underline());
            println!("{}", msg.content)
        }
    }

    Some(())
}

pub async fn run_shell<T>(session: &mut Session<T>, thread_id: uuid::Uuid) -> anyhow::Result<()>
where
    T: Write,
{
    let mut thread = session
        .thread_by_id(thread_id)
        .ok_or_else(|| format_err!("Thread does not exist"))?;

    let mut stdout = io::stdout();
    let mut stdin = io::stdin();

    let mut buf = String::new();

    let mut show_role = true;

    loop {
        if show_role {
            println!("{}", "<User>".green().bold().underline());
        }
        print!("{}", INPUT_INDICATOR);
        stdout.flush()?;

        let input_line = stdin.read_line(&mut buf)?;

        let trimmed_input = buf.trim_start_matches(INPUT_INDICATOR).trim();

        if trimmed_input.is_empty() {
            show_role = false;
            buf.clear();
            continue;
        }

        show_role = true;

        let mut stream = session
            .stream_user_message(trimmed_input, thread_id)
            .await?;
        pin_mut!(stream);

        println!("\n{}", "<Assistant>".blue().bold().underline());

        let mut message_content = String::new();

        while let Some(content) = stream.next().await {
            if let Some(token) = content? {
                message_content.push_str(&token);
                stdout.write_all(token.as_bytes())?;
                stdout.flush()?;
            }
        }

        let new_message = Message {
            content: message_content,
            role: Role::Assistant,
            timestamp: Utc::now(),
        };

        session
            .thread_by_id_mut(thread_id)
            .unwrap()
            .add_message(new_message);

        session.save_to_db()?;

        println!();
    }

    Ok(())
}

pub async fn run_cli() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let mut session = Session::new_dummy()?;
    session.load_threads()?;

    match &cli.command {
        Commands::List => {
            for (i, thread) in session.ordered_threads().iter().map(|(_, x)| x).enumerate() {
                println!(
                    "({}) {} {}",
                    i + 1,
                    thread.init_time().unwrap(),
                    thread.first_message().unwrap().content
                )
            }
        }

        Commands::Resume { index }
            if (*index < 1 || *index > dbg!(session.nonempty_count()) as i64) =>
        {
            return Err(format_err!("Invalid index {index}"))
        }

        Commands::Resume { index } => {
            let thread_id = session
                .ordered_threads()
                .get((index - 1) as usize)
                .map(|(id, _)| id.to_owned())
                .expect("Failed to fetch thread")
                .to_owned();

            let current_thread = session
                .thread_by_id(thread_id)
                .expect("Could not get thread from id");

            clear_screen()?;

            for msg in current_thread.messages.iter() {
                print_message(msg);
            }

            run_shell(&mut session, thread_id).await?;
        }
        Commands::New { prompt } => {
            let prompt_str = match prompt {
                Some(prompt) => prompt.as_str(),
                None => DEFAULT_PROMPT,
            };

            let new_thread_id = session.new_thread(prompt_str)?;

            clear_screen()?;
            run_shell(&mut session, new_thread_id).await?;
        }
    };

    Ok(())
}