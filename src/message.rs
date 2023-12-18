use ansi_to_tui::IntoText;
use anyhow::format_err;
use chrono::{DateTime, Utc};

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, StyledGrapheme, Text};
use serde::{Deserialize, Serialize};
use std::default;
use std::time::{SystemTime, UNIX_EPOCH};
use syntect::easy::HighlightLines;
use syntect::parsing::SyntaxReference;
use syntect::util::LinesWithEndings;

#[allow(unused)]
use itertools::Itertools;

#[allow(unused)]
use futures::StreamExt;

#[allow(unused)]
use futures_util::TryStreamExt;

lazy_static::lazy_static! {

   static ref CODEBLOCK_PATTERN: regex::Regex= regex::RegexBuilder::new(r"```(?<header>\w+)?\n(?<content>.*?)\n```")
        .dot_matches_new_line(true)
        .build()
        .expect("Regex failed to compile");

    static ref SYNTAX_SET: syntect::parsing::SyntaxSet =  syntect::parsing::SyntaxSet::load_defaults_nonewlines();


    static ref THEME_SET: syntect::highlighting::ThemeSet = syntect::highlighting::ThemeSet::load_defaults();


}

const DEFAULT_THEME: &str = "base16-eighties.dark";

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
    pub fn tui_display_header(&self) -> Span {

        
        match self {
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
            Role::System => Span::styled(
                "System",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::UNDERLINED),
            ),
        }
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

    pub fn update(&mut self, text: &str) {
        self.content.push_str(text);
        self.update_blocks();

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
    pub fn formatted_content<'a>(&'a self, index: &mut usize, line_width: u16) -> Text<'a> {
        let mut formatted_lines: Vec<Line> = Vec::new();
        let mut block_index = 0usize;

        for msg_line in self.non_code_content.lines() {
            if msg_line.trim() == BLOCK_MARKER {
                if let Some(block) = self.code_blocks.get(block_index) {
                    formatted_lines
                        .extend(block.highlighted_text(*index, line_width).lines.into_iter());
                    block_index += 1;
                    *index += 1;
                }
            } else {
                formatted_lines.push(msg_line.into());
            }
        }

        Text::from(formatted_lines)
    }

    ///update code_blocks and non_code_content to align with the message text
    pub fn update_blocks(&mut self) {
        let mut blocks = Vec::new();
        self.code_blocks.clear();

        let with_blocks_extracted = CODEBLOCK_PATTERN
            .replace_all(&self.content, |cap: &regex::Captures<'_>| {
                let language = cap.get(1).map(|s| s.as_str().to_owned());

                let content = cap
                    .get(2)
                    .map(|s| s.as_str().to_owned())
                    .unwrap_or_default();

                let block = CodeBlock::new(language, content);

                blocks.push(block);

                BLOCK_MARKER
            })
            .to_string();

        self.code_blocks.clear();
        self.code_blocks.extend(blocks);

        self.non_code_content = with_blocks_extracted;
    }
}

/// collect a group of styled graphemes into equivalent spans
fn coalesce_graphemes<'a, T>(mut graphemes: T) -> Vec<Span<'a>>
where
    T: IntoIterator<Item = StyledGrapheme<'a>>,
{
    graphemes
        .into_iter()
        .group_by(|g| g.style)
        .into_iter()
        .map(|(style, group)| Span::styled(group.map(|g| g.symbol).join(""), style))
        .collect()
}

#[derive(Debug, Clone, Default)]
pub struct CodeBlock {
    pub language: Option<String>,
    pub content: String,
    lines_24_bit_terminal_escaped: Vec<String>,
    lines_tui: Vec<Line<'static>>,
}

impl CodeBlock {
    fn new(language: Option<String>, content: String) -> Self {
        let mut block = Self {
            language,
            content,
            ..Default::default()
        };

        block.update_lines();
        block
    }

    fn update_lines(&mut self) {
        let mut hl = HighlightLines::new(self.syntax(), &THEME_SET.themes[DEFAULT_THEME]);

        let term_lines = self
            .content
            .lines()
            .map(|line| {
                let ranges: Vec<(syntect::highlighting::Style, &str)> =
                    hl.highlight_line(line, &SYNTAX_SET).unwrap();

                syntect::util::as_24_bit_terminal_escaped(&ranges[..], true)
            })
            .collect_vec();

        self.lines_24_bit_terminal_escaped = term_lines;

        self.lines_tui = self
            .lines_24_bit_terminal_escaped
            .iter()
            .map(|s| s.into_text().expect("Text conversion failed"))
            .flat_map(|t| t.lines.into_iter())
            .collect_vec();
    }

    pub fn highlighted_text(&self, index: usize, line_width: u16) -> Text<'_> {
        let mut hl = HighlightLines::new(self.syntax(), &THEME_SET.themes[DEFAULT_THEME]);

        let bg_color = THEME_SET.themes[DEFAULT_THEME].settings.background.map(
            |syntect::highlighting::Color { r, g, b, .. }| ratatui::style::Color::Rgb(r, g, b),
        );

        let pad = StyledGrapheme::new(
            " ",
            Style {
                bg: bg_color,
                ..Default::default()
            },
        );

        let mut formatted_lines: Vec<Line> = Vec::new();

        for line in self.lines_tui.iter() {
            let width_adjusted_lines = line
                .styled_graphemes(Style::default())
                .chunks(line_width.into())
                .into_iter()
                .map(|ln_chunk| ln_chunk.pad_using(line_width.into(), |_| pad.clone()))
                .map(|chunk| Line::from(coalesce_graphemes(chunk)))
                .collect_vec();

            formatted_lines.extend(width_adjusted_lines);
        }

        let annotation: Line = Span::styled(
            format!("({index})"),
            Style::default()
                .add_modifier(Modifier::ITALIC)
                .fg(Color::LightMagenta),
        )
        .into();

        formatted_lines.push(annotation);

        Text::from(formatted_lines)
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
