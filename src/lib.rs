mod db;

mod app;
pub mod tui;

use anyhow::format_err;
use regex::{self, RegexBuilder};
use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{self, json, Value};
use std::borrow::BorrowMut;
use std::collections::HashMap;
use std::io::{self, Stdout, Write};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};
use tokio_stream::StreamExt;
use tokio_util::io::StreamReader;
use uuid::Uuid;

use db::{init_db, DbStore};

const OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";
const MAX_TOKENS: usize = 200;

fn timestamp() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time moves forward")
        .as_secs_f64()
}

#[derive(Debug, Serialize, Deserialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    #[default]
    User,
    System,
    Assistant,
}

impl Role {
    pub fn to_num(&self) -> usize {
        match self {
            Role::System => 1,
            Role::User => 2,
            Role::Assistant => 3,
        }
    }

    pub fn from_num(num: usize) -> anyhow::Result<Self> {
        match num {
            1 => Ok(Role::System),
            2 => Ok(Role::User),
            3 => Ok(Role::Assistant),
            _ => Err(format_err!("Role value must be 1, 2, or 3")),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,

    #[serde(skip)]
    pub timestamp: f64,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct Thread {
    pub messages: Vec<Message>,
    pub model: String,

    #[serde(skip)]
    pub id: Uuid,
}

impl Thread {
    /// Get the prompt used to begin this thread
    pub fn prompt(&self) -> &str {
        self.messages
            .first()
            .expect("At least one message")
            .content
            .as_str()
    }

    pub fn str_id(&self) -> String {
        self.id.as_simple().to_string()
    }

    /// Format this thread as JSON suitible for use with the HTTP API
    pub fn as_json_body(&self) -> Value {
        json!({
            "model" : self.model,
            "messages" : self.messages
                .iter()
                .map(|m| serde_json::to_value(m).unwrap())
                .collect::<Vec<Value>>(),

            "stream" : true,
            "max_tokens": MAX_TOKENS
        })
    }

    fn add_message(&mut self, msg: Message) {
        self.messages.push(msg);
    }
}

/// Create a reqwest::Client with the correct default authorization headers
fn create_client() -> anyhow::Result<Client> {
    let token = env!("OPENAI_API_KEY");

    let headers: HeaderMap = [
        (
            header::AUTHORIZATION,
            HeaderValue::from_str(format!("Bearer {token}").as_str())?,
        ),
        (
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        ),
    ]
    .into_iter()
    .collect();

    Client::builder()
        .default_headers(headers)
        .build()
        .map_err(|e| e.into())
}

#[derive(Deserialize, Serialize, Debug)]
struct CompletionDelta {
    content: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct CompletionChoice {
    delta: CompletionDelta,
    finish_reason: Option<String>,
    index: usize,
}

///Struct representing a chunk from the streaming completions API
#[derive(Serialize, Deserialize, Debug)]
struct CompletionChunk {
    id: String,
    created: usize,
    choices: Vec<CompletionChoice>,
}

impl CompletionChunk {
    pub fn token(&self) -> Option<String> {
        self.choices
            .first()
            .and_then(|c| c.delta.content.to_owned())
    }
}

/// Struct holding state for multiple chat sessions
pub struct Session<T>
where
    T: Write,
{
    writer: Option<T>,
    client: Client,
    pub threads: HashMap<Uuid, Thread>,
    db: rusqlite::Connection,
}

impl Session<Stdout> {
    /// Create a new Session that will write its output to stdout
    pub fn new_stdout() -> anyhow::Result<Session<Stdout>> {
        Ok(Session {
            writer: Some(io::stdout()),
            client: create_client()?,
            threads: HashMap::new(),
            db: init_db()?,
        })
    }
}

impl<T> Session<T>
where
    T: Write,
{
    pub fn new(writer: T) -> anyhow::Result<Self> {
        Ok(Self {
            writer: Some(writer),
            client: create_client()?,
            threads: HashMap::new(),
            db: init_db()?,
        })
    }

    pub fn load_threads(&mut self) -> anyhow::Result<()> {
        let loaded_threads = Thread::get_all(&mut self.db)?
            .into_iter()
            .map(|t| (t.id, t))
            .collect();

        self.threads = loaded_threads;

        Ok(())
    }

    pub fn writer(&mut self) -> Option<&mut T> {
        if let Some(out_writer) = self.writer.as_mut() {
            let ptr = out_writer.borrow_mut();
            Some(ptr)
        } else {
            None
        }
    }

    fn add_thread_message(&mut self, id: Uuid, message: Message) -> anyhow::Result<()> {
        self.thread_by_id(id)
            .ok_or(anyhow::format_err!("{id} is not a thread id"))?
            .add_message(message);

        Ok(())
    }

    /// Create a new thread with the given prompt.
    /// Returns a unique ID that can be used to access the thread
    pub fn new_thread(&mut self, prompt: &str) -> anyhow::Result<Uuid> {
        let messages = vec![Message {
            role: Role::System,
            content: prompt.into(),
            timestamp: timestamp(),
        }];

        let model = "gpt-4".into();
        let id = Uuid::new_v4();

        let thread = Thread {
            messages,
            model,
            id,
        };

        if self.threads.insert(id, thread).is_some() {
            Err(anyhow::format_err!("Thread ID was already present: {id}"))
        } else {
            Ok(id)
        }
    }

    /// Get a mutable reference to a thread from its id
    pub fn thread_by_id(&mut self, id: Uuid) -> Option<&mut Thread> {
        self.threads.get_mut(&id)
    }

    /// Main interface method to send a message to a thread and await a reply.
    /// The response is written to the session's writer, and saved to the thread state.
    pub async fn send_user_message(&mut self, msg: &str, thread_id: Uuid) -> anyhow::Result<()> {
        if msg.trim().is_empty() {
            return Ok(());
        }

        let user_message = Message {
            role: Role::User,
            content: msg.trim().into(),
            timestamp: timestamp(),
        };

        let data = {
            let thread = self
                .thread_by_id(thread_id)
                .ok_or(anyhow::format_err!("No thread found with id: {thread_id}"))?;

            thread.add_message(user_message);

            thread.as_json_body()
        };

        let response = self.client.post(OPENAI_URL).json(&data).send().await?;

        let stream = response
            .error_for_status()?
            .bytes_stream()
            .map(|e| e.map_err(|e| tokio::io::Error::new(tokio::io::ErrorKind::Other, e)));

        let mut chunk_buffer = Vec::new();
        let mut message_content_buf = std::io::BufWriter::new(Vec::new());
        let mut stream_reader = StreamReader::new(stream);

        let buffered_reader = BufReader::new(&mut stream_reader);
        let mut lines = buffered_reader.lines();

        // regex to remove the 'data: ' prefix on the chunks
        let pat = RegexBuilder::new(r"^\s*data:")
            .multi_line(true)
            .build()
            .unwrap();

        while let Some(line) = lines.next_line().await? {
            let line = pat.replace(line.trim(), "");
            if !line.is_empty() {
                match serde_json::from_str::<Value>(&line) {
                    Ok(chunk) => {
                        let completion_chunk: CompletionChunk = serde_json::from_value(chunk)?;

                        if let Some(token) = completion_chunk.token() {
                            if let Some(output_writer) = self.writer() {
                                output_writer.write_all(token.as_bytes())?;
                                output_writer.flush()?;
                            }

                            message_content_buf.write_all(token.as_bytes())?;
                        }

                        chunk_buffer.clear();
                    }

                    Err(_) => {
                        chunk_buffer.push(line.to_string());
                    }
                }
            }
        }

        message_content_buf.flush()?;

        let content = String::from_utf8(message_content_buf.into_inner()?)?;

        let asst_message = Message {
            role: Role::Assistant,
            content,
            timestamp: timestamp(),
        };

        self.add_thread_message(thread_id, asst_message)?;

        self.save_to_db()?;

        Ok(())
    }

    pub async fn run_shell<R>(&mut self, thread_id: Uuid, reader: &mut R) -> anyhow::Result<()>
    where
        R: AsyncBufRead + std::marker::Unpin,
    {
        let mut buf = String::new();

        loop {
            reader.read_line(&mut buf).await?;

            if buf.is_empty() {
                continue;
            }

            if buf.to_lowercase().trim() == "q" {
                break;
            }

            self.send_user_message(&buf, thread_id).await?;

            if let Some(writer) = self.writer() {
                writer.write_all("\n".as_bytes())?;
                writer.flush()?;
            }

            buf.clear();
        }

        Ok(())
    }

    pub fn save_to_db(&mut self) -> anyhow::Result<()> {
        for thread in self.threads.values() {
            thread.to_db(&mut self.db)?;
        }

        Ok(())
    }
}

impl<T> Drop for Session<T>
where
    T: Write,
{
    fn drop(&mut self) {
        self.save_to_db().unwrap();
    }
}
