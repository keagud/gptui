use crate::config::CONFIG;
use crate::session::{Role,  Thread};
use anyhow::format_err;
use crossbeam_channel::bounded;
use crossbeam_channel::Receiver;
use futures::StreamExt;
use futures_util::TryStreamExt;
use itertools::Itertools;
use reqwest::blocking::Client as BlockingClient;
use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::Client as AsyncClient;
use serde::{Deserialize, Serialize};
use serde_json::{self, json};

const OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";
pub trait HttpClient: Sized {
    fn init() -> crate::Result<Self>;
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
            fn init() -> crate::Result<Self> {
                build_client!()
            }
        }
    };
}
impl_client!(AsyncClient);
impl_client!(BlockingClient);

/// Create a reqwest::Client with the correct default authorization headers
pub fn create_client<T>() -> crate::Result<T>
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

fn try_parse_chunks(input: &str) -> crate::Result<(Option<Vec<CompletionChunk>>, Option<String>)> {
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
pub fn stream_thread_reply(thread: &Thread) -> crate::Result<Receiver<Option<String>>> {
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

pub fn fetch_thread_name(thread: &Thread) -> crate::Result<String> {
    let client = create_client::<BlockingClient>()?;

    let chat_content = thread
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
