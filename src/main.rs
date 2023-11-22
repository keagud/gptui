#![allow(dead_code)]
use std::collections::HashMap;
use std::time::Duration;

use anyhow::format_err;
use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use serde_json::{self, json};

const OPENAI_API_KEY: &str = env!("OPENAI_API_KEY");
const OPENAI_API_URL: &str = "https://api.openai.com/v1";

const ALONZO_ID: &str = "asst_dmPg6sGBpzXbVrWOxafSTC9Q";

const POLL_INTERVAL_SEC: usize = 2;

macro_rules! openai_url {
    ($s:literal) => {
        concat!("https://api.openai.com/v1", $s)
    };

    ($s:expr) => {
        format!("https://api.openai.com/v1{}", $s).as_str()
    };

    ($fmt:literal, $($v:expr),*) => {

        {
            let _s = format!($fmt, $($v),* );
            format!("https://api.openai.com/v1{}", _s)
        }

    };
}

macro_rules! val_or_err {
    ($s:expr) => {{
        $s.ok_or_else(|| anyhow::format_err!("Can't convert to type"))
    }};
}

fn make_headers() -> HeaderMap {
    let header_pairs = [
        (
            header::AUTHORIZATION,
            concat!("Bearer ", env!("OPENAI_API_KEY")),
        ),
        (header::CONTENT_TYPE, "application/json"),
        (HeaderName::from_static("openai-beta"), "assistants=v1"),
    ];

    header_pairs
        .into_iter()
        .map(|(k, v)| {
            let val = HeaderValue::from_str(v).unwrap();
            (k, val)
        })
        .collect()
}

fn init_client() -> anyhow::Result<Client> {
    let client = Client::builder().default_headers(make_headers()).build()?;
    Ok(client)
}

async fn create_thread(client: &Client) -> anyhow::Result<String> {
    let req = client.post(openai_url!("/threads"));

    let response = req.send().await?.error_for_status()?;

    let json_reply: serde_json::Value = serde_json::from_str(response.text().await?.as_str())?;

    let thread_id = json_reply
        .as_object()
        .ok_or(format_err!(
            "Unable to parse reply to thread creation request"
        ))?
        .get("id")
        .and_then(|val| val.as_str())
        .ok_or(format_err!("Reply was missing thread id"))?;

    Ok(thread_id.into())
}

#[derive(Serialize, Deserialize, Debug, Default)]
struct Assistant {
    id: String,
    name: String,
    description: Option<String>,
}

#[derive(Debug, Serialize)]
pub enum Role {
    User,
    Assistant,
}

impl<'de> Deserialize<'de> for Role {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s: String = Deserialize::deserialize(deserializer)?;

        match s.to_ascii_lowercase().as_str().trim() {
            "user" => Ok(Role::User),
            "assistant" => Ok(Role::Assistant),
            _ => {
                let msg = format!(
                    "Invalid role value '{}', expected 'user' or 'assistant' ",
                    s
                );

                Err(serde::de::Error::custom(&msg))
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Message {
    id: String,
    created_at: usize,
    thread_id: String,
    text_content: String,
    role: Role,
    assistant_id: Option<String>,
}

impl Message {
    pub fn from_json_text(json_text: &str) -> anyhow::Result<Self> {
        let json_value: serde_json::Value = serde_json::from_str(json_text)?;
        Self::from_json_reply(&json_value)
    }
    pub fn from_json_reply(json_value: &serde_json::Value) -> anyhow::Result<Self> {
        let vals_map = json_value
            .as_object()
            .ok_or(format_err!("Invalid json object"))?;

        let get_val = |key: &str| -> anyhow::Result<&serde_json::Value> {
            let val = vals_map
                .get(key)
                .ok_or(format_err!("Could not get expected value {key}"))?;

            Ok(val)
        };

        let id: String = val_or_err!(get_val("id")?.as_str())?.into();
        let created_at: usize = val_or_err!(get_val("created_at")?.as_u64())? as usize;
        let thread_id: String = val_or_err!(get_val("thread_id")?.as_str())?.into();
        let role = {
            let role_str = val_or_err!(get_val("role")?.as_str())?;

            match role_str.to_ascii_lowercase().trim() {
                "user" => Ok(Role::User),
                "assistant" => Ok(Role::Assistant),

                _ => Err(format_err!(
                    "Invalid role value '{}', expected 'user' or 'assistant'",
                    role_str
                )),
            }
        }?;

        let assistant_id = vals_map
            .get("assistant_id")
            .and_then(|v| v.as_str())
            .and_then(|s| {
                if s == "null" {
                    None
                } else {
                    Some(s.to_string())
                }
            });

        let text_content = {
            let content_arr = val_or_err!(get_val("content")?.as_array())?
                .iter()
                .filter_map(|obj| {
                    if let Some(text_content) = obj.as_object()?.get("text") {
                        text_content
                            .as_object()?
                            .get("value")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_owned())
                    } else {
                        None
                    }
                })
                .collect::<Vec<String>>();

            content_arr.join(" ").to_string()
        };

        Ok(Message {
            id,
            created_at,
            thread_id,
            role,
            assistant_id,
            text_content,
        })
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct Run {
    id: String,
    created_at: usize,
    status: String,
}

struct Thread {
    pub id: String,
    pub messages: Vec<Message>,
    pub assistant: Assistant,
    client: Client,
    poll_interval: usize,
}

impl Thread {
    pub fn new(id: &str, assistant: Assistant) -> anyhow::Result<Self> {
        let client = init_client()?;
        Ok(Thread {
            id: id.into(),
            assistant,
            messages: Vec::new(),
            client,
            poll_interval: POLL_INTERVAL_SEC,
        })
    }

    /// Make a new thread associated with the assistant
    pub async fn create(assistant: Assistant) -> anyhow::Result<Self> {
        let client = init_client()?;
        let id = create_thread(&client).await?;

        Ok(Thread {
            id,
            assistant,
            client,
            messages: Vec::new(),
            poll_interval: POLL_INTERVAL_SEC,
        })
    }

    /// Get the state of this thread from the API, and update this struct's state to match.
    pub async fn fetch(&mut self) -> anyhow::Result<&mut Vec<Message>> {
        let reply_json = self
            .client
            .get(openai_url!("/threads/{}/messages", self.id))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        let json_val: serde_json::Value = serde_json::from_str(&reply_json)?;

        let mut messages: Vec<Message> = json_val
            .as_object()
            .and_then(|obj| obj.get("data"))
            .and_then(|arr| arr.as_array())
            .ok_or(format_err!("Cannot parse messages from JSON"))?
            .iter()
            .map(Message::from_json_reply)
            .collect::<anyhow::Result<Vec<Message>>>()?;

        messages.sort_by_key(|m| m.created_at);

        self.messages = messages;
        Ok(&mut self.messages)
    }

    /// Submit the current state of the thread for a run.
    /// Return the Run object
    async fn submit(&self) -> anyhow::Result<Run> {
        let request_json_str = json!({"assistant_id" : &self.assistant.id }).to_string();

        let reply_json_str = self
            .client
            .post(openai_url!("/threads/{}/runs", &self.id))
            .body(request_json_str)
            .send()
            .await?
            .text()
            .await?;

        let run: Run = serde_json::from_str(&reply_json_str)?;

        Ok(run)
    }

    /// Send the message to this thread, and wait asynchronously
    /// for it to finish.
    /// Return a reference to the completed message.
    pub async fn await_reply(&mut self, message: &str) -> anyhow::Result<&Message> {
        self.add_message(message).await?;
        let run = self.submit().await?;

        let sleep_duration = Duration::from_secs(self.poll_interval as u64);

        let check_url = openai_url!("/threads/{}/runs/{}", self.id, run.id).to_owned();

        loop {
            let reply = self
                .client
                .get(&check_url)
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;

            let run_update: Run = serde_json::from_str(&reply)?;

            match run_update.status.to_ascii_lowercase().trim() {
                "completed" => break,
                "queued" => (),
                "in_progress" => (),
                _ => return Err(format_err!("Run status is {}", run_update.status)),
            }

            sleep(sleep_duration).await;
        }

        Ok(self
            .fetch()
            .await?
            .last()
            .expect("Thread should not be empty"))
    }

    /// Add a message to the thread without running or submitting.
    async fn add_message(&self, message: &str) -> anyhow::Result<Message> {
        let request_json_str = json!({"role" : "user", "content": message}).to_string();

        let reply_json = self
            .client
            .post(openai_url!("/threads/{}/messages", self.id))
            .body(request_json_str)
            .send()
            .await?
            .text()
            .await?;

        let new_msg = Message::from_json_text(&reply_json)?;
        Ok(new_msg)
    }
}

struct Session {
    threads: HashMap<String, Thread>,
}

impl Session {
    pub fn init() -> anyhow::Result<Self> {
        let threads = HashMap::new();

        Ok(Self { threads })
    }

    pub async fn create_thread(&mut self, assistant: Assistant) -> anyhow::Result<&mut Thread> {
        let new_thread = Thread::create(assistant).await?;
        let new_thread_id = new_thread.id.to_owned();
        self.threads.insert(new_thread_id.clone(), new_thread);
        Ok(self.threads.get_mut(&new_thread_id).unwrap())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let asst = Assistant {
        id: ALONZO_ID.into(),
        name: "Alonzo".into(),
        description: Some("A programming helper".into()),
    };

    let mut session = Session::init()?;

    let thread = session.create_thread(asst).await?;
    let reply = thread.await_reply("How to I exit vim?").await?;

    println!("{}", reply.text_content);

    Ok(())
}
