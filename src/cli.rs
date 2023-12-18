use crate::{
    config::{Prompt, CONFIG},
    session::{Message, Role, Session},
};
use anyhow;
use anyhow::format_err;

use clap::{Parser, Subcommand};
use crossterm::{
    self,
    terminal::{Clear, ClearType},
};
use futures::StreamExt;

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
    New {
        #[arg(short, long, help = "Prompt to use")]
        prompt: Option<String>,
    },
    Resume {
        index: i64,
    },
}

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

pub fn run_cli() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let mut session = Session::new()?;
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

            let mut app = App::with_thread(session, thread_id)?;
            app.run()?;
        }
        Commands::New { prompt } => {
            let prompt = match prompt {
                Some(prompt_label) => CONFIG
                    .get_prompt(prompt_label)
                    .ok_or_else(|| format_err!("No prompt found with label '{prompt_label}'"))?.to_owned(),
                None => Prompt::default(),
            };

            let new_thread_id = session.new_thread(&prompt)?;

            let mut app = App::with_thread(session, new_thread_id)?;
            app.run()?;
        }
    };

    Ok(())
}
