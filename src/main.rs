use std::collections::HashMap;

use anyhow;
use anyhow::format_err;
use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Body, Client, Method};
use serde::{Deserialize, Serialize};

use serde_json::{self, json};

const OPENAI_API_KEY: &'static str = env!("OPENAI_API_KEY");
const OPENAI_API_URL: &'static str = "https://api.openai.com/v1";

const ALONZO_ID: &'static str = "asst_dmPg6sGBpzXbVrWOxafSTC9Q";

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

    let response = req.send().await?.text().await?;

    let json_reply: serde_json::Value = serde_json::from_str(&response.as_str())?;

    let thread_id = json_reply
        .as_object()
        .ok_or(format_err!(
            "Unable to parse reply to thread creation request"
        ))?
        .get("id".into())
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
                .get(key.into())
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
            .get("assistant_id".into())
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
                .into_iter()
                .filter_map(|obj| {
                    if let Some(text_content) = obj.as_object()?.get("text".into()) {
                        text_content
                            .as_object()?
                            .get("value".into())
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

struct Thread {
    id: String,
    pub messages: Vec<Message>,
    pub assistant: Assistant,
    client: Client,
}

impl Thread {
    pub fn new(id: &str, assistant: Assistant) -> anyhow::Result<Self> {
        let client = init_client()?;
        Ok(Thread {
            id: id.into(),
            assistant,
            messages: Vec::new(),
            client,
        })
    }

    pub async fn create(assistant: Assistant) -> anyhow::Result<Self> {
        let client = init_client()?;
        let id = create_thread(&client).await?;

        Ok(Thread {
            id,
            assistant,
            client,
            messages: Vec::new(),
        })
    }

    pub async fn fetch(&mut self) -> anyhow::Result<&Vec<Message>> {
        let reply_json = self
            .client
            .get(openai_url!("/threads/{}/messages", self.id))
            .send()
            .await?
            .text()
            .await?;

        let json_val: serde_json::Value = serde_json::from_str(&reply_json)?;

        let messages: Vec<Message> = json_val
            .as_object()
            .and_then(|obj| obj.get("data".into()))
            .and_then(|arr| arr.as_array())
            .ok_or(format_err!("Cannot parse messages from JSON"))?
            .into_iter()
            .map(|val| Message::from_json_reply(val))
            .collect::<anyhow::Result<Vec<Message>>>()?;

        self.messages = messages;
        Ok(&self.messages)
    }
}

struct Session {
    client: Client,
    threads: HashMap<String, Thread>,
}

impl Session {
    pub fn init() -> anyhow::Result<Self> {
        let client = init_client()?;
        let threads = HashMap::new();

        Ok(Self { client, threads })
    }

    pub async fn create_thread(&mut self, assistant: Assistant) -> anyhow::Result<String> {
        let req = self.client.post(openai_url!("/threads"));

        let response = req.send().await?.text().await?;

        let json_reply: serde_json::Value = serde_json::from_str(&response.as_str())?;

        let thread_id: String = json_reply
            .as_object()
            .ok_or(format_err!(
                "Unable to parse reply to thread creation request"
            ))?
            .get("id".into())
            .and_then(|val| val.as_str())
            .ok_or(format_err!("Reply was missing thread id"))?
            .into();

        let new_thread = Thread::new(&thread_id, assistant)?;
        self.threads.insert(thread_id.clone(), new_thread);

        Ok(thread_id)
    }

    pub async fn send_message(
        &mut self,
        thread_id: &str,
        message_text: &str,
    ) -> anyhow::Result<&Message> {
        let thread = self
            .threads
            .get_mut(thread_id)
            .ok_or(format_err!("Not a known thread id: {thread_id}"))?;

        let message_json = json!( {
                "role" : "user",
                "content" : message_text
        });

        let req = self
            .client
            .post(openai_url!("/threads/{}/messages", thread_id))
            .body(message_json.to_string());

        let json_text = req.send().await?.text().await?;

        let new_message = Message::from_json_text(&json_text)?;

        thread.messages.push(new_message);

        Ok(thread
            .messages
            .last()
            .expect("Message was added successfuly"))
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
    let thread_id = session.create_thread(asst).await?;
    let new_msg = session
        .send_message(&thread_id, "How do I exit vim?")
        .await?;

    println!("{:?}", new_msg);

    Ok(())
}
