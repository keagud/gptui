use std::cell::RefCell;
use std::process;
use std::{borrow::Cow, process::Stdio};

use crate::editor::input_from_editor;
use anyhow::format_err;
use crossbeam_channel::Receiver;
use edit::get_editor;
use itertools::Itertools;

use ratatui::{
    prelude::{Alignment, Constraint, CrosstermBackend, Direction, Layout},
    style::{Color, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};

use serde::de::value::CowStrDeserializer;
use uuid::Uuid;

use crossterm::{
    event::{
        self,
        Event::{self},
        KeyCode::{self},
        KeyEvent, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

use ctrlc::set_handler;

use crate::clip;
use crate::session::{stream_thread_reply, Message, Role, Session, Thread};

type ReplyRx = Receiver<Option<String>>;

type Backend = ratatui::backend::CrosstermBackend<std::io::Stderr>;
type CrosstermTerminal = ratatui::Terminal<Backend>;

const FPS: f64 = 30.0;
const SCROLL_STEP: usize = 1;

fn extend_text<'a>(text1: Text<'a>, text2: Text<'a>) -> Text<'a> {
    let mut t = text1.clone();
    t.extend(text2.lines);
    t
}

macro_rules! concat_text {
    ($t1:expr,$t2:expr) => {{
        extend_text($t1, $t2)
    }};

    ($t1:expr, $t2:expr, $($rest:expr),+) => {
        { concat_text!(concat_text!($t1, $t2), $($rest),+) }

    };
}

pub struct App {
    should_quit: bool,
    session: Session,
    thread_id: Option<uuid::Uuid>,
    reply_rx: Option<ReplyRx>,
    user_message: String,
    tick_duration: std::time::Duration,
    chat_scroll: usize,
    bottom_text: Option<String>,
    copy_select_buf: String,
    copy_mode: bool,
    selected_block_index: Option<usize>,
    content_line_width: u16,
    text_len: usize,
    chat_window_height: u16,
    should_show_editor: bool,
}

macro_rules! resolve_thread_id {
    (Some($thread_id:expr)) => {
        Some($thread_id)
    };

    (None) => {
        None
    };

    ($thread_id:expr) => {
        Some($thread_id)
    };
}

macro_rules! app_defaults {
    () => {{
        match Session::new() {
            Ok(s) => app_defaults!(s),
            Err(e) => Err(e),
        }
    }};

    ($session:expr, $thread_id:ident) => {{
        let tick_duration = std::time::Duration::from_secs_f64(1.0 / FPS);

        Ok(Self {
            should_quit: false,
            session: $session,
            thread_id: resolve_thread_id!($thread_id),
            reply_rx: Default::default(),
            user_message: String::new(),
            chat_scroll: 0,
            text_len: 0,
            tick_duration,
            bottom_text: None,
            copy_select_buf: String::new(),
            copy_mode: false,
            selected_block_index: None,
            content_line_width: 0,
            should_show_editor: false,
            chat_window_height: 0,
        })
    }};

    ($session:expr) => {{
        app_defaults!($session, None)
    }};
}

macro_rules! thread_missing {
    ($opt:expr) => {
        $opt.ok_or_else(|| anyhow::format_err!("Not connected to a thread"))
    };
}

// get an initial slice of a string, ending with elipsis,
//desired_length is the maximum final length including elipsis.
fn string_preview(text: &str, desired_length: usize) -> Cow<'_, str> {
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

impl App {
    fn messages(&self) -> anyhow::Result<Vec<&Message>> {
        self.thread().map(|t| t.messages())
    }
    fn thread(&self) -> anyhow::Result<&Thread> {
        thread_missing! {
        self.thread_id.and_then(|id| self.session.thread_by_id(id))
        }
    }

    fn thread_mut(&mut self) -> anyhow::Result<&mut Thread> {
        thread_missing! {
            self
        .thread_id
        .and_then(|id| self.session.thread_by_id_mut(id))
        }
    }

    pub fn startup() -> anyhow::Result<()> {
        enable_raw_mode()?;
        execute!(std::io::stderr(), EnterAlternateScreen)?;
        Ok(())
    }

    pub fn shutdown() -> anyhow::Result<()> {
        execute!(std::io::stderr(), LeaveAlternateScreen)?;
        disable_raw_mode()?;
        Ok(())
    }

    fn max_scroll(&self) -> usize {
        if (self.chat_window_height as usize) > self.text_len {
            0usize
        } else {
            self.text_len
                .saturating_sub(self.chat_window_height as usize)
        }
    }

    fn visible_text<'a>(&self, text: impl Into<Text<'a>>) -> Text<'a> {
        let text: Text<'_> = text.into();

        text.lines
            .into_iter()
            .skip(self.chat_scroll)
            .take(self.chat_window_height.into())
            .collect_vec()
            .into()
    }

    fn scroll_down(&mut self) {
        let max_scroll = self.text_len;
    }

    /// helper function to clear the copy buffer and unset the copy mode state
    fn exit_copy_mode(&mut self) {
        self.copy_select_buf.clear();
        self.copy_mode = false;
    }

    /// 'minor mode' allowing the user to select code block text by its displayed index
    fn update_copy_mode(&mut self, key_event: KeyEvent) -> anyhow::Result<()> {
        match key_event.code {
            KeyCode::Esc => self.exit_copy_mode(),
            KeyCode::Enter => {
                if let Some(index) = self.selected_block_index {
                    match self.thread()?.code_blocks().get(index.saturating_sub(1)) {
                        None => {
                            self.bottom_text = Some(format!("No selection for '{}'!", index));
                            self.exit_copy_mode();
                        }
                        Some(block) => {
                            clip::copy(&block.content)?;
                            self.bottom_text =
                                Some(format!("Copied '{}'", string_preview(&block.content, 30)));
                            // TODO extract this cleanup logic to a function
                            self.exit_copy_mode();
                        }
                    }
                }
            }

            KeyCode::Char(c) if c.is_ascii_digit() => {
                self.copy_select_buf.push(c);

                match self.copy_select_buf.parse::<usize>() {
                    Ok(n)
                        if self
                            .thread()?
                            .code_blocks()
                            .get(n.saturating_sub(1))
                            .is_some() =>
                    {
                        self.selected_block_index = Some(n);
                        self.bottom_text = None;
                    }

                    Ok(m) => {
                        self.bottom_text = Some(format!("No selection for '{}'!", m));
                        self.copy_mode = false;
                        self.copy_select_buf.clear();
                    }

                    _ => {
                        self.copy_mode = false;
                        self.copy_select_buf.clear();
                    }
                }
            }

            _ => (),
        }

        Ok(())
    }

    fn update_awaiting_send(&mut self) -> anyhow::Result<()> {
        if let Event::Key(
            key_event @ KeyEvent {
                kind: event::KeyEventKind::Press,
                code: key_code,
                modifiers: key_modifiers,
                ..
            },
        ) = crossterm::event::read()?
        {
            match key_code {
                // ctrl-c to quit
                KeyCode::Char('c') if matches!(key_modifiers, KeyModifiers::CONTROL) => {
                    self.should_quit = true;
                }

                //scroll history up
                KeyCode::Up => {
                    self.chat_scroll = self.chat_scroll.saturating_sub(SCROLL_STEP);
                }

                // scroll history down
                KeyCode::Down => {
                    self.chat_scroll = self
                        .chat_scroll
                        .saturating_add(SCROLL_STEP)
                        .clamp(0, self.max_scroll());
                }

                // if already in copy mode, forward event to its handler
                _ if self.copy_mode => self.update_copy_mode(key_event)?,

                // ctrl-w to enter copy mode
                KeyCode::Char('w') if matches!(key_modifiers, KeyModifiers::CONTROL) => {
                    self.copy_mode = true;
                }

                // Open an external editor
                KeyCode::Char('e') if matches!(key_modifiers, KeyModifiers::CONTROL) => {
                    self.should_show_editor = true;
                }
                //submit the message with alt-enter
                KeyCode::Enter if matches!(key_modifiers, KeyModifiers::ALT) => {
                    if !self.user_message.is_empty() {
                        let new_message = Message::new_user(&self.user_message);
                        self.thread_mut()?.add_message(new_message);

                        self.reply_rx = Some(stream_thread_reply(self.thread()?)?);

                        self.user_message.clear();
                    }
                }

                // insert a newline
                KeyCode::Enter => {
                    self.user_message.push('\n');
                }

                // enter uppercase char
                KeyCode::Char(c) if matches!(key_modifiers, KeyModifiers::SHIFT) => {
                    self.user_message.push(c.to_ascii_uppercase());
                }

                // enter lowercase char
                KeyCode::Char(c) => {
                    self.user_message.push(c);
                }

                // delete last char
                KeyCode::Backspace => {
                    self.user_message.pop();
                }

                _ => (),
            }
        }

        Ok(())
    }

    fn is_recieving(&self) -> bool {
        self.reply_rx.is_some()
    }

    fn update_recieving(&mut self) -> anyhow::Result<()> {
        self.chat_scroll = self.max_scroll();
        if let Some(rx) = self.reply_rx.as_ref() {
            {
                match rx.recv()? {
                    Some(s) => {
                        self.thread_mut()?.update(&s);
                    }
                    None => {
                        self.thread_mut()?.commit_message();

                        if let Some(a) = self.reply_rx.take() {
                            drop(a)
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn update(&mut self) -> anyhow::Result<()> {
        let has_key_input = crossterm::event::poll(self.tick_duration)?;

        match self.reply_rx {
            None if has_key_input => {
                self.update_awaiting_send()?;
            }

            Some(_) => {
                // flush keyboard input while recieving
                if has_key_input {
                    let _ = crossterm::event::read();
                }
                self.update_recieving()?;
            }

            _ => (),
        }

        Ok(())
    }

    fn ui(&mut self, frame: &mut Frame) -> anyhow::Result<()> {
        let h_padding = 5u16;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([Constraint::Percentage(80), Constraint::Percentage(20)].as_ref())
            .split(frame.size());

        self.content_line_width = chunks[0].width - (h_padding * 2) - 2;

        let mut msgs_formatted = self
            .thread()?
            .tui_formatted_messages(self.content_line_width);

        let msg_lines = msgs_formatted
            .into_iter()
            .flat_map(|m| m.lines)
            .collect_vec();

        let text_len = msg_lines.len();

        let window_height = chunks[0].height;

        let msgs_text = self.visible_text(msg_lines);

        let (border_color, border_type) = if self.copy_mode {
            (Color::Magenta, BorderType::Thick)
        } else {
            (Color::default(), BorderType::Rounded)
        };

        let box_color = if self.is_recieving() {
            Style::new().white()
        } else {
            Style::new().blue()
        };

        let first_msg = self
            .thread()?
            .first_message()
            .map(|m| m.content.as_str())
            .unwrap_or("");

        let chat_window = Paragraph::new(msgs_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(border_type)
                    .border_style(Style::default().fg(border_color))
                    .title(string_preview(first_msg, self.content_line_width.into()).to_string())
                    .padding(ratatui::widgets::Padding {
                        left: h_padding,
                        right: h_padding,
                        ..Default::default()
                    }),
            )
            .wrap(Wrap { trim: false });

        frame.render_widget(chat_window, chunks[0]);

        let mut input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(box_color)
            .border_type(ratatui::widgets::BorderType::Rounded);

        let alert_msg = self
            .bottom_text
            .as_deref()
            .map(|t| t.to_string())
            .or_else(|| self.selected_block_index.map(|i| i.to_string()));

        if let Some(alert_msg) = alert_msg {
            input_block = input_block
                .title(string_preview(&alert_msg, self.content_line_width.into()).to_string())
                .title_alignment(Alignment::Left)
                .title_style(Style::default().cyan())
                .title_position(ratatui::widgets::block::Position::Bottom);
        }

        let input_widget = Paragraph::new(self.user_message.as_str())
            .wrap(Wrap { trim: false })
            .block(input_block);

        frame.render_widget(input_widget, chunks[1]);

        self.text_len = text_len;
        self.chat_window_height = chunks[0].height;
        Ok(())
    }

    pub fn with_thread(session: Session, thread_id: Uuid) -> anyhow::Result<Self> {
        app_defaults!(session, thread_id)
    }

    pub fn new(prompt: &str) -> anyhow::Result<Self> {
        let mut session = Session::new()?;

        app_defaults!(session)
    }

    pub fn run(&mut self) -> anyhow::Result<()> {
        // set_handler(|| {
        //     App::shutdown().expect("Cleanup procedure failed");
        // })?;

        App::startup()?;

        let mut terminal = CrosstermTerminal::new(CrosstermBackend::new(std::io::stderr()))?;

        // initial draw to initialize internal ui state variables
        self.update()?;
        terminal.draw(|frame| self.ui(frame).unwrap())?;
        self.chat_scroll = self.max_scroll();

        while !self.should_quit {
            self.update()?;

            terminal.draw(|frame| self.ui(frame).unwrap())?;

            if self.should_show_editor {
                self.show_editor(&mut terminal)?;
                self.should_show_editor = false;
            }
        }

        App::shutdown()?;
        Ok(())
    }

    fn show_editor(&mut self, terminal: &mut CrosstermTerminal) -> anyhow::Result<()> {
        terminal.clear()?;
        terminal.flush()?;

        if let Some(editor_input) = input_from_editor(&self.user_message)? {
            self.user_message = editor_input;
        }

        App::startup()?;
        terminal.clear()?;
        Ok(())
    }
}

#[cfg(test)]
mod test_app {
    use super::*;

    #[test]
    fn test_string_preview() {
        assert_eq!(
            string_preview("llorum ipsum dolor sit amet", 9),
            "llorum..."
        )
    }
}
