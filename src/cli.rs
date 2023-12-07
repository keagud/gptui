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

pub async fn run_shell<T>(session: &mut Session<T>, thread_id: uuid::Uuid) -> anyhow::Result<()>
where
    T: Write,
{
    let mut thread = session
        .thread_by_id(thread_id)
        .ok_or_else(|| format_err!("Thread does not exist"))?;

    let mut code_blocks: Vec<CodeBlock> = Vec::new();

    let mut code_block_counter = 1usize;

    clear_screen()?;

    for msg in thread.messages().iter() {
        match msg.role {
            Role::System => continue,
            Role::User => {
                println!("{}", "<User>".green().bold().underline());
                println!("{}{}", INPUT_INDICATOR, msg.content);
            }

            Role::Assistant => {
                println!("{}", "<Assistant>".blue().bold().underline());

                let (annotated_content, blocks) =
                    msg.get_content_annotations(&mut code_block_counter);

                println!("{}", annotated_content);

                if let Some(blocks) = blocks {
                    code_blocks.extend(blocks.into_iter());
                }
            }
        }
    }

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

        // holds the raw message, applies no formatting changes
        // this is what is stored in the db
        let mut message_raw_content = String::new();

        // holds the message as present in the output buffer
        // including alterations to formatting
        let mut message_printed_content = String::new();

        //state flag for if we're in the middle of a code block
        let mut is_block = false;

        let mut block_bytes = 0usize;
        let mut block_start = 0usize;

        while let Some(content) = stream.next().await {
            if let Some(token) = content? {
                message_raw_content.push_str(&token);
                message_printed_content.push_str(&token);

                stdout.write_all(token.as_bytes())?;

                // state flag for if we're at the boundary of a code block
                let block_border = message_raw_content.trim().ends_with(BLOCK_DELIMITER);

                match (is_block, block_border) {
                    // when entering a block,
                    // record the byte offset from the start as block_start
                    (false, true) => {
                        is_block = true;
                        block_start = message_printed_content.len() - token.len();
                    }
                    (true, false) => (),

                    // End of a block.
                    (true, true) => {
                        is_block = false;

                        // get a slice from the start of the block to the end of
                        // the output buffer's contents
                        let block_slice = &message_printed_content[block_start..];

                        // parse the slice into a CodeBlock struct
                        let language = block_slice
                            .lines()
                            .next()
                            .and_then(|ln| ln.strip_prefix(BLOCK_DELIMITER))
                            .and_then(|s| {
                                if s.is_empty() {
                                    None
                                } else {
                                    Some(s.trim().to_string())
                                }
                            });

                        let content = block_slice
                            .lines()
                            .skip(1)
                            .take_while(|ln| ln.trim() != BLOCK_DELIMITER)
                            .join("\n");

                        let code_block = CodeBlock { language, content };

                        delete_bytes_back(block_slice.len() as u16)?;
                        io::stdout().flush()?;

                        let block_pretty_printed =
                            code_block.pretty_print_str(code_blocks.len() + 1)?;

                        io::stdout().write_all(block_pretty_printed.as_bytes())?;
                        io::stdout().flush()?;

                        // remove the unformatted block from the raw content buffer,
                        // and replace it with the pretty printed one.
                        // This is necessary to calculate the byte offsets for the next block
                        // otherwise we'd be out of sync with the actual output as seen.
                        message_raw_content = message_raw_content[..block_start].into();
                        message_raw_content.push_str(&block_pretty_printed);

                        // add the code block to the collection
                        code_blocks.push(code_block);
                    }
                    _ => (),
                }

                stdout.flush()?;
            }
        }

        // add the new message to this thread and session
        // then save to the db
        let new_message = Message {
            content: message_raw_content,
            role: Role::Assistant,
            timestamp: Utc::now(),
            ..Default::default()
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

            run_shell(&mut session, thread_id).await?;
        }
        Commands::New { prompt } => {
            let prompt_str = match prompt {
                Some(prompt) => prompt.as_str(),
                None => DEFAULT_PROMPT,
            };

            let new_thread_id = session.new_thread(prompt_str)?;

            run_shell(&mut session, new_thread_id).await?;
        }
    };

    Ok(())
}
