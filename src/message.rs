use ansi_to_tui::IntoText;
use anyhow::format_err;
use chrono::{DateTime, Utc};
use itertools::Itertools;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use syntect::easy::HighlightLines;
use syntect::parsing::SyntaxReference;
use syntect::util::LinesWithEndings;

#[allow(unused)]
use futures::StreamExt;

#[allow(unused)]
use futures_util::TryStreamExt;

lazy_static::lazy_static! {

   static ref CODEBLOCK_PATTERN: regex::Regex= regex::RegexBuilder::new(r"```(?<header>\w+)?(?<content>.*?)```")
        .dot_matches_new_line(true)
        .build()
        .expect("Premade regex should be ok");

    static ref SYNTAX_SET: syntect::parsing::SyntaxSet =  syntect::parsing::SyntaxSet::load_defaults_newlines();


    static ref THEME_SET: syntect::highlighting::ThemeSet = syntect::highlighting::ThemeSet::load_defaults();


}

const DEFAULT_THEME: &str = "base16-ocean.dark";

#[allow(unused)]
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
    non_code_content: String,
}

const BLOCK_MARKER: &str = "```__<BLOCK>__```";
impl Message {
    pub fn code_blocks(&self) -> Vec<&CodeBlock> {
        self.code_blocks.iter().collect()
    }

    pub fn non_code_content(&self) -> &str {
        &self.non_code_content
    }

    pub fn new(role: Role, content: &str, timestamp: DateTime<Utc>) -> Self {
        let mut new_msg = Self {
            role,
            content: content.into(),
            timestamp,
            ..Default::default()
        };

        new_msg.update_blocks();

        new_msg
    }

    pub fn new_user(text: &str) -> Self {
        let role = Role::User;
        let timestamp = Utc::now();
        Self::new(role, text, timestamp)
    }

    pub fn new_asst(text: &str) -> Self {
        let role = Role::Assistant;
        let timestamp = Utc::now();
        Self::new(role, text, timestamp)
    }

    pub fn new_from_db(role: Role, content: String, timestamp_epoch: f64) -> Self {
        let timestamp_secs = f64::floor(timestamp_epoch) as i64;
        let timestamp_nanos = f64::fract(timestamp_epoch) * 1_000_000f64;

        let timestamp = DateTime::from_timestamp(timestamp_secs, timestamp_nanos.floor() as u32)
            .expect("Epoch time was valid");

        Self::new(role, &content, timestamp)
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

        for msg_line in self.non_code_content.lines() {
            if msg_line.trim() == BLOCK_MARKER {
                if let Some(block) = self.code_blocks.get(block_index) {
                    formatted_lines.extend(block.highlighted_text(*index)?.lines.into_iter());
                    block_index += 1;
                }
            } else {
                formatted_lines.push(msg_line.into());
            }
        }

        Ok(Text::from(formatted_lines))
    }

    // TODO make sure this is called on every new message initializiation
    ///update code_blocks and non_code_content to align with the message text
    pub fn update_blocks(&mut self) {
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

                let _lang = if let Some(ref s) = block.language {
                    s
                } else {
                    ""
                };

                blocks.push(block);

                BLOCK_MARKER
            })
            .to_string();

        self.code_blocks.clear();
        self.code_blocks.extend(blocks);

        self.non_code_content = with_blocks_extracted;
    }
}

#[derive(Debug, Clone)]
pub struct CodeBlock {
    pub language: Option<String>,
    pub content: String,
}

impl CodeBlock {
    pub fn highlighted_text(&self, _index: usize) -> anyhow::Result<Text<'_>> {
        let mut hl = HighlightLines::new(self.syntax(), &THEME_SET.themes[DEFAULT_THEME]);

        let mut formatted_lines: Vec<Line> = Vec::new();

        #[allow(unused)]
        let line_indents = self.content.lines().map(|ln| {
            ln.chars()
                .take_while(|c| c.is_whitespace())
                .map(|c| match c {
                    ' ' => 1,
                    '\t' => 2,
                    _ => 0,
                })
                .sum::<usize>()
        });

        for line in LinesWithEndings::from(&self.content) {
            let ranges: Vec<(syntect::highlighting::Style, &str)> =
                hl.highlight_line(line, &SYNTAX_SET).unwrap();
            let escaped = syntect::util::as_24_bit_terminal_escaped(&ranges[..], true);

            let e = escaped.into_text()?;

            formatted_lines.extend(e.lines.into_iter());

            //            formatted_lines.extend(e.into_iter());

            // let line_spans = hl
            //     .highlight_line(line, &SYNTAX_SET)?
            //     .into_iter()
            //     .filter_map(|segment| into_span(segment).ok())
            //     .collect_vec();

            // let line_hl = Line::from(line_spans);
            // formatted_lines.push(line_hl);
        }

        Ok(Text::from(formatted_lines))
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
