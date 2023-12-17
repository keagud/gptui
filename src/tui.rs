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
    terminal: RefCell<CrosstermTerminal>,
    should_quit: bool,
    session: Session,
    thread_id: Option<uuid::Uuid>,
    reply_rx: Option<ReplyRx>,
    incoming_message: String,
    user_message: String,
    tick_duration: std::time::Duration,
    chat_scroll: usize,
    bottom_text: Option<String>,
    copy_select_buf: String,
    copy_mode: bool,
    selected_block_index: Option<usize>,
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

        match CrosstermTerminal::new(CrosstermBackend::new(std::io::stderr())) {
            Ok(term) => Ok(Self {
                should_quit: false,
                session: $session,
                thread_id: resolve_thread_id!($thread_id),
                reply_rx: Default::default(),
                incoming_message: String::new(),
                user_message: String::new(),
                chat_scroll: 0,
                tick_duration,
                bottom_text: None,
                copy_select_buf: String::new(),
                copy_mode: false,
                selected_block_index: None,
                terminal: RefCell::new(term),
            }),

            Err(e) => Err(anyhow::anyhow!(e)),
        }
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
                    self.chat_scroll = self.chat_scroll.saturating_add(SCROLL_STEP);
                }

                // if already in copy mode, forward event to its handler
                _ if self.copy_mode => self.update_copy_mode(key_event)?,

                // ctrl-w to enter copy mode
                KeyCode::Char('w') if matches!(key_modifiers, KeyModifiers::CONTROL) => {
                    self.copy_mode = true;
                }

                // Open an external editor
                KeyCode::Char('e') if matches!(key_modifiers, KeyModifiers::CONTROL) => {
                    self.show_editor()?;
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
        if let Some(rx) = self.reply_rx.as_ref() {
            {
                match rx.recv()? {
                    Some(s) => {
                        if s.contains('\n') {
                            self.chat_scroll += SCROLL_STEP;
                        }

                        self.incoming_message.push_str(&s)
                    }
                    None => {
                        let new_msg = Message::new_asst(&self.incoming_message);
                        self.thread_mut()?.add_message(new_msg);
                        self.incoming_message.clear();

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
                self.update_recieving()?;
            }

            _ => (),
        }

        Ok(())
    }

    fn ui(&self, frame: &mut Frame) -> anyhow::Result<()> {
        let h_padding = 5u16;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([Constraint::Percentage(80), Constraint::Percentage(20)].as_ref())
            .split(frame.size());

        let content_line_width = chunks[0].width - (h_padding * 2) - 2;

        let first_msg = self
            .thread()?
            .first_message()
            .map(|m| m.content.as_str())
            .unwrap_or("");

        let mut msgs_formatted = self.thread()?.tui_formatted_messages(content_line_width);

        if self.is_recieving() {
            let mut incoming_lines = vec![Line::from(vec![
                Role::Assistant.tui_display_header(),
                "\n".into(),
            ])];

            incoming_lines.extend(self.incoming_message.lines().map(Line::from));

            msgs_formatted.push(Text::from(incoming_lines));
        }

        let box_color = if self.is_recieving() {
            Style::new().white()
        } else {
            Style::new().blue()
        };

        let msg_lines = msgs_formatted
            .into_iter()
            .flat_map(|m| m.lines)
            .collect_vec();

        let msgs_text = Text::from(msg_lines);

        let window_height = chunks[0].height as usize;

        let scroll: u16 = if window_height > msgs_text.height() {
            // if window is larger than text, no need to scroll
            0usize
        } else {
            self.chat_scroll
        } as u16;

        let (border_color, border_type) = if self.copy_mode {
            (Color::Magenta, BorderType::Thick)
        } else {
            (Color::default(), BorderType::Rounded)
        };

        let chat_window = Paragraph::new(msgs_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(border_type)
                    .border_style(Style::default().fg(border_color))
                    .title(string_preview(first_msg, content_line_width.into()).to_string())
                    .padding(ratatui::widgets::Padding {
                        left: h_padding,
                        right: h_padding,
                        ..Default::default()
                    }),
            )
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0));

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
                .title(string_preview(&alert_msg, content_line_width.into()).to_string())
                .title_alignment(Alignment::Left)
                .title_style(Style::default().cyan())
                .title_position(ratatui::widgets::block::Position::Bottom);
        }

        let input_widget = Paragraph::new(self.user_message.as_str())
            .wrap(Wrap { trim: false })
            .block(input_block);

        frame.render_widget(input_widget, chunks[1]);

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

        while !self.should_quit {
            self.update()?;

            self.terminal
                .borrow_mut()
                .draw(|frame| self.ui(frame).unwrap())?;
        }

        App::shutdown()?;
        Ok(())
    }

    fn show_editor(&mut self) -> anyhow::Result<()> {
        self.terminal.borrow_mut().clear()?;
        self.terminal.borrow_mut().flush()?;

        if let Some(editor_input) = input_from_editor()? {
            self.user_message = editor_input;
        }

        App::startup()?;
        self.terminal.borrow_mut().clear()?;
        Ok(())
    }
}

fn editor_binary() -> anyhow::Result<String> {
    #[cfg(target_family = "windows")]
    let editor = get_editor().map(|s| s.to_string_lossy().into())?;

    #[cfg(target_family = "unix")]
    let editor =
        std::env::var("EDITOR").or_else(|_| get_editor().map(|s| s.to_string_lossy().into()))?;

    #[cfg(not(any(target_family = "unix", target_family = "windows")))]
    compile_error!("Unsupported compile target");

    Ok(editor)
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
