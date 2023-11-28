use regex::{self, RegexBuilder};
use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::{Client};
use serde::{Deserialize, Serialize};
use serde_json::{self, json, Value};
use std::io::{self, Write};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_stream::StreamExt;
use tokio_util::io::StreamReader;

const OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";

enum Role {
    User,
    System,
}

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

async fn get_reply(msg: &str, output_writer: &mut impl Write) -> anyhow::Result<()> {
    let client = create_client()?;

    let data = json!({
        "model" : "gpt-4",
        "messages" : [
         {
            "role" : "system",
            "content" : "You are a helpful assistant"
        },
        {
            "role" : "user",
            "content" : msg
        }
    ],
        "stream" : true,
        "max_tokens" : 50
    });

    let response = client
        .post(OPENAI_URL)
        .json(&data)
        .send()
        .await?
        .error_for_status()?;

    let mut buffer = Vec::new();

    let stream = response
        .bytes_stream()
        .map(|e| e.map_err(tokio::io::Error::other));

    let mut stream_reader = StreamReader::new(stream);

    let buffered_reader = BufReader::new(&mut stream_reader);
    let mut lines = buffered_reader.lines();

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
                        output_writer.write_all(token.as_bytes())?;
                        output_writer.flush()?;
                    }

                    buffer.clear();
                }

                Err(_) => {
                    buffer.push(line.to_string());
                }
            }
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let msg = "Come up with 10 possible pun-based names for a fertility clinic that is also a video game store.";

    let mut stdout = io::stdout();

    get_reply(msg, &mut stdout).await?;

    Ok(())
}
