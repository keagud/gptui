use crate::config::{Prompt, CONFIG};
use crate::db::{init_db, DbError, DbStore};
pub use crate::message::{CodeBlock, Message, Role};

use anyhow::format_err;
use chrono::{DateTime, Utc};
use crossbeam_channel::bounded;
use crossbeam_channel::Receiver;
use futures::StreamExt;
use futures_util::TryStreamExt;
use itertools::Itertools;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::text::{Line, Text};
use reqwest::blocking::Client as BlockingClient;
use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::Client as AsyncClient;
use serde::{Deserialize, Serialize};
use serde_json::{self, json, Value};
use std::borrow::Cow;
use std::collections::HashMap;
use std::str::FromStr;
use uuid::Uuid;

const OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    DatabaseError(#[from] DbError),

    #[error("HTTP request returned status {status}")]
    HttpError {
        base_err: reqwest::Error,
        status: reqwest::StatusCode,
        message: String,
    },

    #[error("Connection error: {base_err}")]
    ConnectionError { base_err: reqwest::Error },

    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Sync + Send>),
}

impl From<anyhow::Error> for SessionError {
    fn from(value: anyhow::Error) -> Self {
        Self::Other(value.into())
    }
}

impl From<reqwest::Error> for SessionError {
    fn from(value: reqwest::Error) -> Self {
        if let Some(status) = value.status() {
            let message = value.to_string();
            Self::HttpError {
                base_err: value,
                status,
                message,
            }
        } else {
            Self::ConnectionError { base_err: value }
        }
    }
}

pub type SessionResult<T> = Result<T, SessionError>;

// get an initial slice of a string, ending with elipsis,
//desired_length is the maximum final length including elipsis.
pub fn string_preview(text: &str, desired_length: usize) -> Cow<'_, str> {
    if text.len() <= desired_length {
        return text.into();
    }

    Cow::from(
        text.chars()
            .take(desired_length.saturating_sub(3))
            .chain("...".chars())
            .take(desired_length)
            .join(""),
    )
}
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct Thread {
    messages: Vec<Message>,
    pub model: String,

    #[serde(skip)]
    pub id: Uuid,

    #[serde(skip)]
    prompt: Prompt,

    #[serde(skip)]
    incoming: Option<Message>,

    #[serde(skip)]
    thread_title: Option<String>,
}

impl Thread {
    pub fn new(messages: Vec<Message>, model: &str, id: Uuid) -> Self {
        Self {
            messages,
            model: model.into(),
            id,
            ..Default::default()
        }
    }

    pub fn thread_title(&self) -> Option<&str> {
        self.thread_title.as_ref().map(|s| s.as_ref())
    }

    pub fn display_title(&self) -> String {
        let title = self
            .thread_title()
            .or_else(|| self.first_message().map(|m| m.content.as_str()))
            .unwrap_or("...");

        string_preview(title, 100).to_string()
    }

    pub fn set_title(&mut self, title: &str) {
        self.thread_title = Some(title.into())
    }

    pub fn list_preview(&self) -> Option<String> {
        let local_time_fmt = self
            .init_time()?
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M");

        let preview_msg = if let Some(title) = self.thread_title() {
            title.to_string()
        } else {
            self.first_message()
                .map(|m| string_preview(&m.content, 200).to_string())?
        };

        Some(format!("{} {}", local_time_fmt, preview_msg))
    }
    pub fn message_display_header(&self, role: Role) -> Span {
        match role {
            Role::User => Span::styled(
                "User",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::UNDERLINED),
            ),
            Role::Assistant => {
                let color = Color::from_str(self.prompt().color().unwrap_or("blue"))
                    .expect("Could not parse color from string");

                Span::styled(
                    self.prompt().label(),
                    Style::default()
                        .fg(color)
                        .add_modifier(Modifier::UNDERLINED),
                )
            }
            Role::System => Span::styled(
                "System",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::UNDERLINED),
            ),
        }
    }

    pub fn non_sys_messages(&self) -> Vec<&Message> {
        self.messages
            .iter()
            .filter(|m| !m.is_system())
            .collect_vec()
    }

    pub fn messages(&self) -> Vec<&Message> {
        self.messages.iter().collect()
    }
    /// Get the prompt used to begin this thread
    pub fn prompt(&self) -> &Prompt {
        &self.prompt
    }

    pub fn code_blocks(&self) -> Vec<&CodeBlock> {
        self.messages.iter().flat_map(|m| m.code_blocks()).collect()
    }

    pub fn set_incoming_message(&mut self, text: &str) {
        self.incoming = Some(Message::new_asst(text));
    }

    /// Add token(s) to the incoming message in progress
    pub fn update(&mut self, incoming_text: &str) {
        if self.incoming.is_some() {
            if let Some(m) = self.incoming.as_mut() {
                m.update(incoming_text)
            }
        } else {
            self.incoming = Some(Message::new_asst(incoming_text));
        };
    }

    /// Commit the completed message to the thread, and reset state for the next incoming message
    pub fn commit_message(&mut self) -> SessionResult<()> {
        if let Some(msg) = self.incoming.take() {
            self.messages.push(msg);

            if self.thread_title().is_none() && self.non_sys_messages().len() >= 2 {
                self.update_thread_name()?;
            }
        }

        Ok(())
    }
    pub fn clear_incoming_message(&mut self) {
        self.incoming = None;
    }

    /// Get all messages in this thread as they will be displayed
    pub fn tui_formatted_messages(&self, line_width: u16) -> Vec<Text> {
        let mut msgs_buf: Vec<Text> = Vec::new();
        let mut block_counter = 1usize;
        let mut all_blocks = Vec::new();

        for msg in self
            .messages
            .iter()
            .map(Some)
            .chain(std::iter::once(self.incoming.as_ref()))
            .flatten()
            .filter(|m| !m.is_system())
        {
            let header_line = Line::from(vec![self.message_display_header(msg.role)]);

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

    pub fn update_thread_name(&mut self) -> SessionResult<()> {
        self.thread_title = Some(self.fetch_thread_name()?);
        Ok(())
    }

    pub fn fetch_thread_name(&self) -> SessionResult<String> {
        let client = create_client::<BlockingClient>()?;

        let chat_content = self
            .messages()
            .iter()
            .filter(|m| !m.is_system())
            .map(|m| {
                let msg_label = match m.role {
                    Role::Assistant => "Assistant",
                    Role::User => "User",
                    _ => unreachable!(),
                };

                format!("{}:\n{}\n", msg_label, &m.content)
            })
            .join("\n");

        let prompt = r"
        Your task is to provide brief descriptive titles to message threads. 
        Each title should be no more than 100 characters in length.
        Your response should consist of the title and nothing else.";

        let body = json!({
        "model" : "gpt-3.5-turbo",
        "messages": [
            {
            "role" : "system",
            "content" : prompt
            },
            {
                "role" : "user",
                "content" : &chat_content
            }]
        });

        let response: serde_json::Value = client.post(OPENAI_URL).json(&body).send()?.json()?;

        let title = response
            .pointer("/choices/0/message/content")
            .and_then(|s| s.as_str())
            .ok_or(format_err!("Could not parse JSON response"))?;

        Ok(title.into())
    }
}

macro_rules! build_client {
    () => {{
        let headers: HeaderMap = [
            (
                header::AUTHORIZATION,
                HeaderValue::from_str(format!("Bearer {}", CONFIG.api_key()).as_str())
                    .expect("Failed to format auth headers"),
            ),
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]
        .into_iter()
        .collect();

        Self::builder()
            .default_headers(headers)
            .build()
            .map_err(|e| e.into())
    }};
}

macro_rules! impl_client {
    ($struct:ident) => {
        impl HttpClient for $struct {
            fn init() -> SessionResult<Self> {
                build_client!()
            }
        }
    };
}

trait HttpClient: Sized {
    fn init() -> SessionResult<Self>;
}

impl_client!(AsyncClient);
impl_client!(BlockingClient);

/// Create a reqwest::Client with the correct default authorization headers
fn create_client<T>() -> SessionResult<T>
where
    T: HttpClient,
{
    T::init()
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

fn try_parse_chunks(input: &str) -> SessionResult<(Option<Vec<CompletionChunk>>, Option<String>)> {
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

            Err(e) => return Err(anyhow::anyhow!(e).into()),
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
    pub fn new() -> SessionResult<Self> {
        Ok(Self {
            threads: HashMap::new(),
            db: init_db()?,
        })
    }

    pub fn load_threads(&mut self) -> SessionResult<()> {
        let loaded_threads = Thread::get_all(&mut self.db)?
            .into_iter()
            .map(|t| (t.id, t))
            .collect();

        self.threads = loaded_threads;

        Ok(())
    }

    pub fn delete_thread(&mut self, thread_id: Uuid) -> SessionResult<bool> {
        if let Some(thread) = self.threads.remove(&thread_id) {
            thread.drop_from_db(&mut self.db)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Create a new thread with the given prompt.
    /// Returns a unique ID that can be used to access the thread
    pub fn new_thread(&mut self, prompt: &Prompt) -> SessionResult<Uuid> {
        let messages = vec![Message::new(Role::System, prompt.prompt(), Utc::now())];

        let model = "gpt-4";
        let id = Uuid::new_v4();

        let mut thread = Thread::new(messages, model, id);
        thread.prompt = prompt.clone();

        if self.threads.insert(id, thread).is_some() {
            Err(anyhow::format_err!("Thread ID was already present: {id}").into())
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
            .filter(|(_, t)| !t.non_sys_messages().is_empty())
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

    pub fn save_to_db(&mut self) -> SessionResult<()> {
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

pub fn stream_thread_reply(thread: &Thread) -> SessionResult<Receiver<Option<String>>> {
    if !thread.last_message().map(|m| m.is_user()).unwrap_or(false) {
        return Err(anyhow::format_err!(
            "The most recent messege in the thread must be from a user"
        )
        .into());
    }

    let client = create_client::<AsyncClient>()?;

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
                            tx.send(Some(s))?;
                        }
                    }
                }
            }

            tx.send(None)?;

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
