use gpt::{self, Assistant, Session};
use std::io::{self, Write};

use clap::{command, Command, Parser, Subcommand};
use std::time::Duration;
use tokio::time::sleep;

pub use gpt::data_dir;

const ALONZO_ID: &str = "asst_dmPg6sGBpzXbVrWOxafSTC9Q";
macro_rules! spinner {
    ($b:block) => {{
        let mut _spinner =
            spinners::Spinner::new(spinners::Spinners::Dots, std::string::String::new());
        let _value = { $b };
        _spinner.stop_with_newline();
        _value
    }};
}

pub enum Assistants {
    ALONZO,
}

async fn print_formatted_reply(text: &str) -> anyhow::Result<()> {
    let delay = Duration::from_millis(100);

    // TODO change the render width to add a margin
    let formatted = termimad::term_text(text);

    let mut stdout = io::stdout();

    for chunk in formatted.to_string().as_bytes().chunks(8) {
        stdout.write(chunk)?;
        stdout.flush()?;

        sleep(delay).await
    }

    stdout.flush()?;
    Ok(())
}

struct ChatSession {
    assistant: Assistant,
    session: Session,
}

impl ChatSession {
    pub async fn new(assistant: Assistant) -> anyhow::Result<Self> {
        let session = Session::load()?;

        Ok(Self { session, assistant })
    }

    pub async fn run_shell(&mut self) -> anyhow::Result<()> {
        let mut buf = String::new();
        let prompt = ">> ";

        let thread = spinner!({ self.session.create_thread(self.assistant.clone()).await? });
        loop {
            buf.clear();
            print!("{}", prompt);
            io::stdout().flush()?;

            io::stdin().read_line(&mut buf)?;
            let user_input = buf.trim();

            let reply_message = spinner!({ thread.await_reply(user_input).await? });

            print_formatted_reply(reply_message.message_text()).await?;

            io::stdout().flush()?;
        }
    }
}

fn cli() -> Command {
    Command::new("gpt")
        .about("OpenAI Assistants Wrapper CLI")
        .subcommand(Command::new("list"))
        .alias("ls")
}

#[derive(Debug, Parser)]
#[command(name = "gpt")]
#[command(about = "OpenAI Assistants Wrapper CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// List available threads
    #[command(alias = "ls")]
    List,

    /// Attach to a thread in a shell
    #[command(alias = "a")]
    Attach { thread: u64 },

    #[command(alias = "n")]
    New,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let asst = Assistant {
        id: ALONZO_ID.into(),
        name: "Alonzo".into(),
        description: Some("A programming helper".into()),
    };

    let _args = Cli::parse();

    let mut session = ChatSession::new(asst).await?;
    session.run_shell().await?;

    Ok(())
}
