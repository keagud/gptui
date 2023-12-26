use std::{
    io::{self, Write},
    ops::Deref,
};

use crate::{
    config::{Prompt, CONFIG},
    db::DbStore,
    session::Session,
};
use anyhow;
use anyhow::format_err;
use clap::{Parser, Subcommand};
use itertools::Itertools;
use uuid::Uuid;

use crate::tui::App;

#[derive(Parser, Debug)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// List all saved threads
    List,

    /// Start a new conversation thread
    New {
        #[arg(short, long, help = "Prompt to use")]
        prompt: Option<String>,
    },

    /// Resume a previous conversation
    Resume { index: i64 },

    /// Delete a conversation thread permanently
    Delete { index: i64 },

    /// Delete all conversation threads
    Clear,
}

fn thread_by_index(session: &Session, index: i64) -> Option<Uuid> {
    session
        .ordered_threads()
        .get((index - 1) as usize)
        .map(|(id, _)| *id)
        .copied()
}

macro_rules! prompt_yn {

    ($fmt:literal, $($args:expr),+) => {
        {

            print!($fmt, $($args),+ );
            io::stdout().flush()?;

            let mut buf = String::new();
            io::stdin().read_line(&mut buf)?;

            match buf.to_lowercase().trim_end() {
                c if c.is_empty() => Some(false),
                "n" => Some(false),
                "y" => {
                    Some(true)
                }
                _ => {
                    None
                }
            }




    }


    };
}

pub fn run_cli() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let mut session = Session::new()?;
    session.load_threads()?;

    match &cli.command {
        Commands::List => {
            for (i, list_preview) in session
                .ordered_threads()
                .iter()
                .filter_map(|(_, t)| t.list_preview())
                .enumerate()
            {
                println!("({}) {}", i + 1, &list_preview);
            }
        }

        Commands::Resume { index } if (*index < 1 || *index > session.nonempty_count() as i64) => {
            return Err(format_err!("Invalid index {index}"))
        }

        Commands::Resume { index } => {
            let thread_id = thread_by_index(&session, *index).expect("Failed to fetch thread");
            let mut app = App::with_thread(session, thread_id)?;
            app.run()?;
        }
        Commands::New { prompt } => {
            let prompt = match prompt {
                Some(prompt_label) => {
                    let matching_prompts = CONFIG.get_matching_prompts(prompt_label);
                    if let Some(prompt) = matching_prompts.first() {
                        if matching_prompts.len() == 1 {
                            prompt.to_owned().clone()
                        } else {
                            let err_text = [format!(
                                "Ambiguous specifier for prompt, '{}' could refer to:",
                                prompt_label
                            )]
                            .into_iter()
                            .chain(
                                matching_prompts
                                    .into_iter()
                                    .map(|p| format!("\t {}", p.label())),
                            )
                            .join("\n");

                            return Err(format_err!(err_text));
                        }
                    } else {
                        let all_prompts = CONFIG
                            .prompts()
                            .into_iter()
                            .map(|p| format!("\t{}", p.label()))
                            .sorted()
                            .join("\n");

                        return Err(format_err!(
                            "No prompt matched '{}'. Available prompts are:\n{}",
                            prompt_label,
                            &all_prompts
                        ));
                    }
                }

                None => Prompt::default(),
            };

            let new_thread_id = session.new_thread(&prompt)?;

            let mut app = App::with_thread(session, new_thread_id)?;
            app.run()?;
        }

        Commands::Delete { index } => {
            let thread = thread_by_index(&session, *index)
                .map(|id| session.thread_by_id(id))
                .flatten()
                .expect("Failed to fetch thread");

            match prompt_yn!("Delete thread '{}'? (y/N)", thread.display_title()) {
                Some(false) => (),
                Some(true) => {
                    session.delete_thread(thread.id)?;
                    println!("Deleted successfully")
                }
                None => {
                    println!("Must be 'y' or 'n'");
                }
            }
        }
        Commands::Clear => {
            let threads_count = session.ordered_threads().len();
            match prompt_yn!(
                "Delete all {} threads? This cannot be undone! (y/N): ",
                threads_count
            ) {
                Some(true) => {
                    let all_ids = session.threads.keys().copied().collect_vec();
                    for thread_id in all_ids.into_iter() {
                        session.delete_thread(thread_id)?;
                    }
                    println!("Deleted {} threads", threads_count);
                }
                _ => (),
            }
        }
    };

    Ok(())
}
