use chrono::{DateTime, Utc};
use crossbeam_channel::bounded;
use crossbeam_channel::Receiver;
use futures::{Stream, StreamExt};
use futures_util::{pin_mut, TryStreamExt};
use itertools::Itertools;
use ratatui::text::{Line, Text};
use ratatui::Frame;
use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{self, json, Value};
use std::collections::HashMap;
use std::io::{self, sink, Sink, Stdout, Write};
use tokio::io::{AsyncBufRead, AsyncBufReadExt};
use uuid::Uuid;

use crate::db::{init_db, DbStore};
pub use crate::message::{CodeBlock, Message, Role};

const OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";
const MAX_TOKENS: usize = 200;

lazy_static::lazy_static! {
    static ref CLIENT: reqwest::Client = create_client()
        .expect("HTTP client initialization failed");

}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct Thread {
    messages: Vec<Message>,
    pub model: String,

    #[serde(skip)]
    pub id: Uuid,
}

impl Thread {
    pub fn new(messages: Vec<Message>, model: &str, id: Uuid) -> Self {
        let _blocks_count = 1;

        Self {
            messages,
            model: model.into(),
            id,
            ..Default::default()
        }
    }

    pub fn messages(&self) -> Vec<&Message> {
        self.messages.iter().collect()
    }
    /// Get the prompt used to begin this thread
    pub fn prompt(&self) -> &str {
        self.messages
            .first()
            .expect("At least one message")
            .content
            .as_str()
    }

    pub fn code_blocks(&self) -> Vec<&CodeBlock> {
        self.messages.iter().flat_map(|m| m.code_blocks()).collect()
    }

    /// Get all messages in this thread as they will be displayed
    pub fn tui_formatted_messages(&self, line_width: u16) -> Vec<Text> {
        let mut msgs_buf: Vec<Text> = Vec::new();
        let mut block_counter = 1usize;
        let mut all_blocks = Vec::new();

        for msg in self.messages().iter().filter(|m| !m.is_system()) {
            let header_line = Line::from(vec![msg.role.tui_display_header()]);

            let text = msg.formatted_content(&mut block_counter, line_width);

            all_blocks.extend(msg.code_blocks().iter().cloned());

            let amended_lines = [header_line]
                .into_iter()
                .chain(text.lines.into_iter())
                .chain(std::iter::once("\n".into()))
                .collect_vec();

            msgs_buf.push(Text::from(amended_lines));
        }

        msgs_buf
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
           // "max_tokens": MAX_TOKENS
        })
    }

    ///Return the time the first non-system message was sent
    pub fn init_time(&self) -> Option<DateTime<Utc>> {
        self.messages.first().map(|m| m.timestamp)
    }

    pub fn add_message(&mut self, msg: Message) {
        self.messages.push(msg);
    }

    /// Get the first non-system message in this thread
    pub fn first_message(&self) -> Option<&Message> {
        self.messages.iter().find(|m| !m.is_system()).to_owned()
    }

    /// Get the most recent message (could be a system message).
    pub fn last_message(&self) -> Option<&Message> {
        self.messages.iter().last()
    }

    pub fn annotated_messages(&self) {}
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
pub struct Session {
    pub threads: HashMap<Uuid, Thread>,
    db: rusqlite::Connection,
}

fn try_parse_chunks(input: &str) -> anyhow::Result<(Option<Vec<CompletionChunk>>, Option<String>)> {
    let mut valid_chunks = Vec::new();

    let mut remainder = None;

    let input_lines = input
        .lines()
        .map(|ln| ln.trim().trim_start_matches("data:").trim())
        .filter(|ln| !ln.is_empty())
        .collect_vec();

    for (i, line) in input_lines.iter().enumerate() {
        match serde_json::from_str::<CompletionChunk>(line) {
            Ok(chunk) => valid_chunks.push(chunk),
            Err(e) if e.is_eof() => {
                remainder = Some(input_lines[i..].join("\n"));

                break;
            }
            Err(e) if e.is_syntax() && *line == "[DONE]" => break,

            Err(e) => return Err(anyhow::anyhow!(e)),
        }
    }

    let return_chunks = if valid_chunks.is_empty() {
        None
    } else {
        Some(valid_chunks)
    };

    Ok((return_chunks, remainder))
}

impl Session {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
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
    fn add_thread_message(&mut self, id: Uuid, message: Message) -> anyhow::Result<()> {
        self.thread_by_id_mut(id)
            .ok_or(anyhow::format_err!("{id} is not a thread id"))?
            .add_message(message);

        Ok(())
    }

    /// Create a new thread with the given prompt.
    /// Returns a unique ID that can be used to access the thread
    pub fn new_thread(&mut self, prompt: &str) -> anyhow::Result<Uuid> {
        let messages = vec![Message::new(Role::System, prompt, Utc::now())];

        let model = "gpt-4";
        let id = Uuid::new_v4();

        let thread = Thread::new(messages, model, id);
        if self.threads.insert(id, thread).is_some() {
            Err(anyhow::format_err!("Thread ID was already present: {id}"))
        } else {
            Ok(id)
        }
    }

    /// Get an (immutable) reference to a thread from its id
    pub fn thread_by_id(&self, id: Uuid) -> Option<&Thread> {
        self.threads.get(&id)
    }

    /// Get a mutable reference to a thread from its id
    pub fn thread_by_id_mut(&mut self, id: Uuid) -> Option<&mut Thread> {
        self.threads.get_mut(&id)
    }

    /// Get references to the Ids and contents of all non-empty threads,
    /// sorted ascending by creation time.
    pub fn ordered_threads(&self) -> Vec<(&Uuid, &Thread)> {
        self.threads
            .iter()
            .filter(|(_, t)| !t.messages.is_empty())
            .sorted_by_key(|(_, t)| t.init_time().expect("Thread has no messages"))
            .collect_vec()
    }

    /// Get the number of threads in this session with at least one message
    pub fn nonempty_count(&self) -> usize {
        self.threads
            .iter()
            .filter(|(_, t)| !t.messages.is_empty())
            .count()
    }

    pub fn save_to_db(&mut self) -> anyhow::Result<()> {
        for thread in self.threads.values() {
            thread.to_db(&mut self.db)?;
        }

        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.save_to_db().unwrap();
    }
}

pub fn stream_thread_reply(thread: &Thread) -> anyhow::Result<Receiver<Option<String>>> {
    if !thread.last_message().map(|m| m.is_user()).unwrap_or(false) {
        return Err(anyhow::format_err!(
            "The most recent messege in the thread must be from a user"
        ));
    }

    let client = create_client()?;

    let (tx, rx) = bounded(100);

    let thread_json = thread.as_json_body();

    let _ = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Async runtime failed to start");

        let res: anyhow::Result<()> = rt.block_on(async move {
            let response = client.post(OPENAI_URL).json(&thread_json).send().await?;

            let mut stream = response
                .error_for_status()?
                .bytes_stream()
                .map_err(|e| anyhow::anyhow!(e));

            let mut buf = String::new();

            let _message_tokens = String::new();

            while let Some(bytes_result) = stream.next().await {
                buf.push_str(String::from_utf8_lossy(&bytes_result?).as_ref());

                let (parsed, remainder) = try_parse_chunks(&buf)?;

                buf.clear();

                if let Some(remainder) = remainder {
                    buf.push_str(&remainder);
                }

                if let Some(chunks) = parsed {
                    for chunk in chunks.iter() {
                        if let Some(s) = chunk.token() {
                            tx.send(Some(s));
                        }
                    }
                }
            }

            tx.send(None);

            Ok(())
        });

        res.expect("Failed to spawn thread");
    });

    Ok(rx)
}
#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_chunks() {
        let data = r#"
         data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1694268190,"model":"gpt-3.5-turbo-0613", "system_fingerprint": "fp_44709d6fcb", "choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}
data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1694268190,"model":"gpt-3.5-turbo-0613", "system_fingerprint": "fp_44709d6fcb", "choices":[{"index":0,"delta":{"content":"!"},"finish_reason":null}]}
data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1694268190,"model":"gpt-3.5-turbo-0613", "system_fingerprint": "fp_44709d6fcb", "choices":[{"index":0,"delta":{"content":" today"},"finish_reason":null}]}
{"id":"chatcmpl-123","object":"chat.completion.chunk", "c
        "#;

        let (parsed, remaining) = try_parse_chunks(data).unwrap();

        let parsed = parsed.unwrap();
        let remaining = remaining.unwrap();

        assert_eq!(
            remaining.as_str(),
            r#"{"id":"chatcmpl-123","object":"chat.completion.chunk", "c"#
        );

        for (token, expected) in parsed
            .into_iter()
            .map(|chunk| chunk.token())
            .zip(["", "!", " today"].into_iter())
        {
            assert!(token.is_some());
            assert_eq!(token.unwrap().as_str(), expected);
        }
    }
}
