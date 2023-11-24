use anyhow::format_err;

use directories::BaseDirs;
use gpt::{self, Assistant, Session, Thread};

use std::fs::{self};
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::sleep;

const ALONZO_ID: &str = "asst_dmPg6sGBpzXbVrWOxafSTC9Q";

const DATA_DIR_NAME: &str = "gpt_rs";

macro_rules! data_dir {
    () => {{
        directories::BaseDirs::new()
            .ok_or(anyhow::format_err!("Unable to access system directories"))
            .map(|d| std::path::PathBuf::from(d.data_dir()))
    }};
}
macro_rules! spinner {
    ($b:block) => {{
        let mut _spinner =
            spinners::Spinner::new(spinners::Spinners::Dots, std::string::String::new());
        let _value = { $b };
        _spinner.stop_with_newline();
        _value
    }};
}

fn save_thread(thread: &Thread) -> anyhow::Result<()> {
    let threads_dir = data_dir!()?.join("threads");
    let thread_filename = format!("threads/{}.json", thread.id);
    fs::create_dir_all(&threads_dir)?;

    let thread_file = fs::File::create(threads_dir.join(thread_filename))?;

    serde_json::to_writer_pretty(thread_file, &thread.dump_json())?;

    Ok(())
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

    fn save_dir() -> anyhow::Result<PathBuf> {
        let data_dir = BaseDirs::new()
            .ok_or(format_err!("Unable to get user data directory"))?
            .data_dir()
            .to_path_buf()
            .join(DATA_DIR_NAME);

        if !data_dir.try_exists()? {
            fs::create_dir_all(data_dir.as_path())?;
        }

        Ok(data_dir)
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let asst = Assistant {
        id: ALONZO_ID.into(),
        name: "Alonzo".into(),
        description: Some("A programming helper".into()),
    };

    let mut session = ChatSession::new(asst).await?;
    session.run_shell().await?;

    Ok(())
}
