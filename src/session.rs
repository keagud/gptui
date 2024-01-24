use crate::client::spawn_client;
use crate::config::PromptSetting;
use crate::db::{init_db, DbStore};
use crate::llm::LlmModel;
pub use crate::message::{CodeBlock, Message, Role};

// use anyhow::format_err;
use chrono::{DateTime, Utc};

use crossbeam_channel::Receiver;
use itertools::Itertools;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::text::{Line, Text};

use serde_json::{self, json, Value};
use std::borrow::Cow;
use std::collections::HashMap;

use std::str::FromStr;
use uuid::Uuid;

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

#[derive(Debug, Default, Clone)]
pub struct Thread {
    messages: Vec<Message>,
    pub model: LlmModel,

    pub id: Uuid,

    pub summary_entries: Vec<Summary>,

    prompt: PromptSetting,

    incoming: Option<Message>,

    thread_title: Option<String>,
}

impl Thread {
    pub fn new(messages: Vec<Message>, model: LlmModel, id: Uuid) -> Self {
        Self {
            messages,
            model,
            id,
            ..Default::default()
        }
    }

    pub fn thread_title(&self) -> Option<&str> {
        self.thread_title.as_ref().map(|s| s.as_ref())
    }

    /// get the total tokens used for this thread.
    /// Token counts are accurate for LLM messages, but user messages
    /// are a rough guesstimate
    pub fn total_tokens(&self) -> usize {
        self.messages
            .iter()
            .map(|m| m.token_count.unwrap_or(m.content.len() / 4))
            .sum()
    }

    pub fn token_use(&self) -> f64 {
        let max_context = self.prompt().model.max_context() as f64;
        let tokens_used = self.total_tokens() as f64;

        tokens_used / max_context
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

    pub fn add_summary(&mut self, _summary: Summary) {
        todo!();
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
                    &self.prompt().label,
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
    pub fn prompt(&self) -> &PromptSetting {
        &self.prompt
    }

    pub fn code_blocks(&self) -> Vec<&CodeBlock> {
        self.messages.iter().flat_map(|m| m.code_blocks()).collect()
    }

    pub fn set_incoming_message(&mut self, text: &str) {
        self.incoming = Some(Message::new_asst(text));
    }

    /// Add a token to the incoming message in progress
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
    pub fn commit_message(&mut self) -> crate::Result<()> {
        if let Some(msg) = self.incoming.take() {
            self.messages.push(msg);
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

    //TODO test this
    /// Get this thread's messages with any summary edits applied
    pub fn minified_messages(&self) -> Vec<Message> {
        if self.summary_entries.is_empty() {
            return self.messages.to_vec();
        }

        // get only the entries that are not contained in the
        // range of another summary
        let max_coverage_summaries = self
            .summary_entries
            .iter()
            .sorted_by_key(|s| s.start_index)
            .group_by(|s| s.start_index)
            .into_iter()
            .filter_map(|(_, g)| g.max_by_key(|x| x.end_index))
            .collect_vec();

        let mut i = 0usize;
        let mut amended_messages = Vec::new();

        for s in max_coverage_summaries.iter() {
            amended_messages.extend_from_slice(&self.messages[i..s.start_index]);

            let summary_timestamp = self.messages[s.end_index.saturating_sub(1)].timestamp;

            amended_messages.push(s.as_message(summary_timestamp));

            i = s.end_index;
        }

        amended_messages.extend_from_slice(&self.messages[i..]);
        amended_messages
    }

    /// Format this thread as JSON suitible for use with the HTTP API
    pub fn as_json_body(&self) -> Value {
        json!({
            "model" : self.model.to_string(),
            "messages" : self.minified_messages()
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

    pub fn fetch_thread_name(&self) -> crate::Result<Receiver<String>> {
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

        spawn_client(body)
    }

    pub fn fetch_summary(&self) -> crate::Result<Receiver<Summary>> {
        todo!();
    }
}
/// Struct holding state for multiple chat sessions
pub struct Session {
    pub threads: HashMap<Uuid, Thread>,
    db: rusqlite::Connection,
}
impl Session {
    pub fn new() -> crate::Result<Self> {
        Ok(Self {
            threads: HashMap::new(),
            db: init_db()?,
        })
    }

    pub fn load_threads(&mut self) -> crate::Result<()> {
        let loaded_threads = Thread::get_all(&mut self.db)?
            .into_iter()
            .map(|t| (t.id, t))
            .collect();

        self.threads = loaded_threads;

        Ok(())
    }

    pub fn delete_thread(&mut self, thread_id: Uuid) -> crate::Result<bool> {
        if let Some(thread) = self.threads.remove(&thread_id) {
            thread.drop_from_db(&mut self.db)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Create a new thread with the given prompt.
    /// Returns a unique ID that can be used to access the thread
    pub fn new_thread(&mut self, prompt: &PromptSetting) -> crate::Result<Uuid> {
        let messages = vec![Message::new(Role::System, &prompt.prompt, Utc::now())];

        let id = Uuid::new_v4();

        let mut thread = Thread::new(messages, prompt.model, id);
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

    pub fn save_to_db(&mut self) -> crate::Result<()> {
        for thread in self.threads.values() {
            thread.to_db(&mut self.db)?;
        }

        Ok(())
    }
}

#[cfg(feature = "debug-dump")]
impl Session {
    pub fn dump_all(&self) {
        for (_, thread) in self.ordered_threads() {
            thread.dump()
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.save_to_db().unwrap();
    }
}

#[derive(Debug, Clone)]
pub struct Summary {
    _thread_id: Uuid,
    pub start_index: usize,
    pub end_index: usize,
    pub content: String,
}

impl Summary {
    pub fn new(thread_id: Uuid, start_index: usize, end_index: usize, content: &str) -> Self {
        Summary {
            _thread_id: thread_id,
            start_index,
            end_index,
            content: content.into(),
        }
    }

    pub fn as_message(&self, timestamp: DateTime<Utc>) -> Message {
        let msg_content = format!("**Summary of elided messages:**: \n {}", &self.content);
        Message::new(Role::System, &msg_content, timestamp)
    }
}

#[cfg(feature = "debug-dump")]
impl Thread {
    pub fn dump_to_file(&self, dest: impl AsRef<Path>) {
        let json = self.as_json_body();
        let dest_file = PathBuf::from(dest.as_ref());

        let json_content = serde_json::to_string_pretty(&json).expect("Failed to write to json");
        std::fs::write(dest_file, json_content.as_bytes()).unwrap();
    }

    pub fn dump_location(&self) -> PathBuf {
        #[cfg(debug_assertions)]
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_assets");

        #[cfg(not(debug_assertions))]
        let p = std::env::current_dir().expect("Could not get current working dir");

        if !p.exists() {
            std::fs::create_dir_all(&p).expect("Could not create directory");
        }

        let title = self.thread_title().unwrap_or("").to_string();

        let display_time = if let Some(t) = self.init_time() {
            t.to_string()
        } else {
            String::new()
        };

        let file_title = filenamify([display_time, title].join("_"));

        let out_file = p.join(&file_title).with_extension("json");

        dbg!(&out_file);

        out_file
    }

    pub fn dump(&self) {
        self.dump_to_file(&self.dump_location())
    }
}
