use crate::session::{CodeBlock, Message, Role, Session, Thread};
use anyhow;
use anyhow::format_err;
use arboard;
use chrono::Utc;
use clap::{Parser, Subcommand};
use colored::Colorize;
use crossterm::{
    self,
    terminal::{Clear, ClearType},
};
use futures::{Stream, StreamExt};
use futures_util::pin_mut;
use itertools::Itertools;
use std::io::{self, Read, Write};

use crate::tui::App;
const DEFAULT_PROMPT: &str = r#"You are a helpful assistant"#;
const INPUT_INDICATOR: &str = ">> ";
const BLOCK_DELIMITER: &str = r"```";

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

fn pprint(input: &str) {}

fn clear_screen() -> anyhow::Result<()> {
    if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/c", "cls"])
            .status()?;
    } else {
        std::process::Command::new("clear").status()?;
    }

    io::stdout().flush()?;

    Ok(())
}

fn backspace() -> io::Result<()> {
    io::stdout().write_all("\x08".as_bytes())?;

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

pub fn delete_bytes_back(bytes_back: u16) -> anyhow::Result<()> {
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, crossterm::cursor::MoveLeft(bytes_back))?;
    crossterm::execute!(stdout, Clear(ClearType::FromCursorDown))?;
    stdout.flush()?;

    Ok(())
}



pub fn run_cli() -> anyhow::Result<()> {
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

        Commands::Resume { index } if (*index < 1 || *index > session.nonempty_count() as i64) => {
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

            let mut app = App::with_thread(session, thread_id);
            app.run()?;
        }
        Commands::New { prompt } => {
            let prompt_str = match prompt {
                Some(prompt) => prompt.as_str(),
                None => DEFAULT_PROMPT,
            };

            let new_thread_id = session.new_thread(prompt_str)?;

            let mut app = App::with_thread(session, new_thread_id);
            app.run()?;
        }
    };

    Ok(())
}
