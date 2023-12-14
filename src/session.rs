use anyhow::format_err;
use chrono::{DateTime, Utc};
use colored::Colorize;
use crossbeam_channel::bounded;
use crossbeam_channel::{Receiver, Sender};
use futures::{Stream, StreamExt};
use futures_util::{pin_mut, Future, TryStreamExt};
use itertools::{repeat_n, Itertools};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{self, json, Value};
use std::borrow::BorrowMut;
use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{self, sink, Sink, Stdout, Write};
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style as SyntaxStyle, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;
use syntect_tui::into_span;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};
use tokio_util::io::StreamReader;
use uuid::Uuid;

use crate::db::{init_db, DbStore};

lazy_static::lazy_static! {

   static ref CODEBLOCK_PATTERN: regex::Regex= regex::RegexBuilder::new(r"```(?<header>\w+)?(?<content>.*?)```")
        .dot_matches_new_line(true)
        .build()
        .expect("Premade regex should be ok");

    static ref SYNTAX_SET: syntect::parsing::SyntaxSet =  syntect::parsing::SyntaxSet::load_defaults_newlines();


    static ref THEME_SET: syntect::highlighting::ThemeSet = syntect::highlighting::ThemeSet::load_defaults();


}

const DEFAULT_THEME: &str = "base16-eighties.dark";

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
    pub fn tui_display_header(&self) -> Option<Span> {
        let header = match self {
            Role::User => Span::styled(
                "User",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::UNDERLINED),
            ),
            Role::Assistant => Span::styled(
                "Assistant",
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::UNDERLINED),
            ),
            Role::System => return None,
        };

        Some(header)
    }

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

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Message {
    pub role: Role,
    pub content: String,

    #[serde(skip)]
    pub timestamp: DateTime<Utc>,

    #[serde(skip)]
    code_blocks: Vec<CodeBlock>,

    #[serde(skip)]
    prose_content: String,
}

const BLOCK_MARKER: &str = "```__<BLOCK>__```";
impl Message {
    pub fn new_user(text: &str) -> Self {
        let role = Role::User;
        let timestamp = Utc::now();
        let content = text.to_string();

        Self {
            role,
            timestamp,
            content,
            ..Default::default()
        }
    }

    pub fn new_asst(text: &str) -> Self {
        let role = Role::Assistant;
        let timestamp = Utc::now();
        let content = text.to_string();

        Self {
            role,
            timestamp,
            content,
            ..Default::default()
        }
    }


    pub fn new(role: Role, content: String, timestamp_epoch: f64) -> Self {
        let timestamp_secs = f64::floor(timestamp_epoch) as i64;
        let timestamp_nanos = f64::fract(timestamp_epoch) * 1_000_000f64;

        let timestamp = DateTime::from_timestamp(timestamp_secs, timestamp_nanos.floor() as u32)
            .expect("Epoch time was valid");

        Self {
            role,
            content,
            timestamp,
            ..Default::default()
        }
    }

    pub fn timestamp_epoch(&self) -> f64 {
        let subsecs = self.timestamp.timestamp_subsec_millis() as f64;
        let secs = self.timestamp.timestamp() as f64;

        secs + (subsecs / 1000f64)
    }

    pub fn timestamp_millis(&self) -> i64 {
        self.timestamp.timestamp_millis()
    }

    pub fn is_user(&self) -> bool {
        self.role == Role::User
    }
    pub fn is_assistant(&self) -> bool {
        self.role == Role::Assistant
    }
    pub fn is_system(&self) -> bool {
        self.role == Role::System
    }

    /// Get the text for this message as it will be displayed, with highlights and annotations
    /// `index` is the value to start numbering the block annotations from
    pub fn formatted_content<'a>(&'a self, index: &mut usize) -> anyhow::Result<Text<'a>> {
        let mut formatted_lines: Vec<Line> = Vec::new();
        let mut block_index = 0usize;

        for msg_line in self.prose_content.lines() {
            if msg_line.trim() == BLOCK_MARKER {
                if let Some(block) = self.code_blocks.get(block_index) {
                    formatted_lines.extend(block.highlighted_text(*index)?.lines.into_iter());
                }
            } else {
                formatted_lines.push(msg_line.into());
            }
        }

        Ok(Text::from(formatted_lines))
    }

    ///update code_blocks and prose_content to align with the message text
    pub fn update_blocks(&mut self) -> anyhow::Result<(())> {
        let mut blocks = Vec::new();
        self.code_blocks.clear();

        let with_blocks_extracted = CODEBLOCK_PATTERN
            .replace_all(&self.content, |cap: &regex::Captures<'_>| {
                let block = CodeBlock {
                    language: cap.get(1).map(|s| s.as_str().to_owned()),
                    content: cap
                        .get(2)
                        .map(|s| s.as_str().to_owned())
                        .unwrap_or_default(),
                };

                let lang = if let Some(ref s) = block.language {
                    s
                } else {
                    ""
                };

                blocks.push(block);

                BLOCK_MARKER
            })
            .to_string();

        self.code_blocks.clear();
        self.code_blocks.extend(blocks.into_iter());

        self.prose_content = with_blocks_extracted;

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CodeBlock {
    pub language: Option<String>,
    pub content: String,
}

impl CodeBlock {
    pub fn highlighted_text<'a>(&'a self, index: usize) -> anyhow::Result<Text<'a>> {
        let mut hl = HighlightLines::new(self.syntax(), &THEME_SET.themes[DEFAULT_THEME]);

        let mut formatted_lines: Vec<Line> = Vec::new();

        let mut line_indents = self.content.lines().map(|ln| {
            ln.chars()
                .take_while(|c| c.is_whitespace())
                .map(|c| match c {
                    ' ' => 1,
                    '\t' => 2,
                    _ => 0,
                })
                .sum::<usize>()
        });

        for (line, indent) in LinesWithEndings::from(&self.content).zip(line_indents) {
            let line_spans = hl
                .highlight_line(line, &SYNTAX_SET)?
                .into_iter()
                .filter_map(|segment| into_span(segment).ok())
                .collect_vec();

            let line_hl = Line::from(line_spans);
        }

        todo!();
    }

    fn syntax(&self) -> &SyntaxReference {
        self.language
            .as_ref()
            .and_then(|lang| SYNTAX_SET.find_syntax_by_token(lang))
            .or_else(|| {
                self.content
                    .lines()
                    .next()
                    .and_then(|ln| SYNTAX_SET.find_syntax_by_first_line(ln))
            })
            .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text())
    }

    pub fn as_raw(&self) -> String {
        format!(
            "```{}\n{}\n```",
            &self.language.as_deref().unwrap_or(""),
            &self.content
        )
    }
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct Thread {
    messages: Vec<Message>,
    pub model: String,

    #[serde(skip)]
    pub id: Uuid,

    #[serde(skip)]
    code_block_counts: usize,
}

impl Thread {
    pub fn new(messages: Vec<Message>, model: &str, id: Uuid) -> Self {
        let mut blocks_count = 1;

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

    pub fn tui_formatted_messages(&self) -> anyhow::Result<Vec<Text>> {
        let mut msgs_buf: Vec<Text> = Vec::new();
        let mut block_counter = 0usize;
        let mut all_blocks = Vec::new();

        for msg in self.messages().iter().filter(|m| !m.is_system()) {
            let header_line = Line::from(vec![msg.role.tui_display_header().unwrap(), "\n".into()]);

            let text = msg.formatted_content(&mut block_counter)?;

            all_blocks.extend(msg.code_blocks.iter().cloned());

            let amended_lines = [header_line]
                .into_iter()
                .chain(text.lines.into_iter())
                .chain(std::iter::once("\n".into()))
                .collect_vec();

            msgs_buf.push(Text::from(amended_lines));
        }

        Ok(msgs_buf)
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
pub struct Session<T>
where
    T: Write,
{
    writer: T,
    client: Client,
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

impl Session<Stdout> {
    /// Create a new Session that will write its output to stdout
    pub fn new_stdout() -> anyhow::Result<Session<Stdout>> {
        Self::new(io::stdout())
    }
}

impl Session<Sink> {
    /// Create a new Session that will write to a dummy sink (no visible output)
    pub fn new_dummy() -> anyhow::Result<Session<Sink>> {
        Self::new(sink())
    }
}

impl Default for Session<Sink> {
    fn default() -> Self {
        Self::new(sink()).expect("Session initialization failed")
    }
}

impl<T> Session<T>
where
    T: Write,
{
    pub fn new(writer: T) -> anyhow::Result<Self> {
        Ok(Self {
            writer,
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

    fn writer(&mut self) -> &mut T {
        &mut self.writer
    }

    fn write_all_flushed(&mut self, buf: impl AsRef<[u8]>) -> io::Result<()> {
        self.writer.write_all(buf.as_ref())?;
        self.writer.flush()?;

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
        let messages = vec![Message {
            role: Role::System,
            content: prompt.into(),
            timestamp: Utc::now(),
            ..Default::default()
        }];

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

    pub fn nonempty_count(&self) -> usize {
        self.threads
            .iter()
            .filter(|(_, t)| !t.messages.is_empty())
            .count()
    }

    pub async fn stream_user_message(
        &mut self,
        msg: &str,
        thread_id: Uuid,
    ) -> anyhow::Result<impl Stream<Item = anyhow::Result<Option<String>>>> {
        let user_message = Message {
            role: Role::User,
            content: msg.trim().into(),
            timestamp: Utc::now(),
            ..Default::default()
        };

        let data = {
            let thread = self
                .thread_by_id_mut(thread_id)
                .ok_or(anyhow::format_err!("No thread found with id: {thread_id}"))?;

            thread.add_message(user_message);

            thread.as_json_body()
        };

        let response = self.client.post(OPENAI_URL).json(&data).send().await?;

        let stream = response
            .error_for_status()?
            .bytes_stream()
            .map(|e| e.map_err(|e| tokio::io::Error::new(tokio::io::ErrorKind::Other, e)));

        let mut buf = String::new();

        let mut message_tokens = String::new();

        let _stream = async_stream::stream! {

            for await chunk_bytes in stream {

                let chunk = chunk_bytes
                    .map_err(|e| anyhow::anyhow!(e))
                    .and_then(|c| String::from_utf8(c.into())
                    .map_err(|e| anyhow::anyhow!(e)))?;


                    buf.push_str(&chunk);

                    let (parsed, remainder) = try_parse_chunks(&buf)?;

                    buf.clear();

                    if let Some(remainder) = remainder {
                        buf.push_str(&remainder);
                    }


                    if let Some(chunks) = parsed {

                        for chunk in chunks.iter() {
                                if let Some(s) = chunk.token() {
                        }
                                yield Ok(chunk.token());

                        }
                    }

            }




        };
        Ok(_stream)
    }

    pub async fn run_shell_stdin(&mut self, thread_id: Uuid) -> anyhow::Result<()> {
        let mut reader = tokio::io::BufReader::new(tokio::io::stdin());

        self.run_shell(thread_id, &mut reader).await
    }

    pub async fn run_shell<R>(&mut self, thread_id: Uuid, reader: &mut R) -> anyhow::Result<()>
    where
        R: AsyncBufRead + std::marker::Unpin,
    {
        let mut buf = String::new();

        loop {
            reader.read_line(&mut buf).await?;
            let buf_trimmed = buf.trim();

            if buf_trimmed.is_empty() {
                continue;
            }

            if buf_trimmed.to_lowercase() == "q" {
                break;
            }

            let stream = self.stream_user_message(buf_trimmed, thread_id).await?;

            pin_mut!(stream);

            while let Some(Ok(token)) = stream.next().await {
                let write_bytes = token.map(|t| t.as_bytes().to_owned()).unwrap_or_default();

                self.write_all_flushed(&write_bytes)?;
            }

            self.write_all_flushed("\n")?;
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

            let mut message_tokens = String::new();

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
