use crate::session::string_preview;
use crate::{editor::input_from_editor, session::Summary};

use crossbeam_channel::Receiver;
use ctrlc::set_handler;
use itertools::Itertools;
use ratatui::{
    prelude::{Alignment, Constraint, CrosstermBackend, Direction, Layout},
    style::{Color, Style, Stylize},
    text::{Span, Text},
    widgets::{
        block::{Position, Title},
        Block, BorderType, Borders, Paragraph, Wrap,
    },
    Frame,
};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture,
        Event::{self},
        KeyCode::{self},
        KeyEvent, KeyModifiers, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use uuid::Uuid;

use crate::client::stream_thread_reply;
use crate::clip;
use crate::session::{Message, Session, Thread};
type ReplyRx = Receiver<Option<String>>;

type Backend = ratatui::backend::CrosstermBackend<std::io::Stderr>;
type CrosstermTerminal = ratatui::Terminal<Backend>;

const FPS: f64 = 30.0;
const SCROLL_STEP: usize = 1;

pub struct App {
    should_quit: bool,
    session: Session,
    thread_id: uuid::Uuid,
    reply_rx: Option<Receiver<Option<String>>>,
    summary_rx: Option<Receiver<Summary>>,
    title_rx: Option<Receiver<String>>,
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
    chat_title: Option<String>,
    token_count: usize,
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
            thread_id: $thread_id,
            reply_rx: Default::default(),
            summary_rx: None,
            title_rx: None,
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
            token_count: 0,
            chat_title: None,
        })
    }};

    ($session:expr) => {{
        app_defaults!($session, None)
    }};
}

impl App {
    fn thread(&self) -> &Thread {
        self.session
            .thread_by_id(self.thread_id)
            .expect("Could not load active thread")
    }

    fn thread_mut(&mut self) -> &mut Thread {
        self.session
            .thread_by_id_mut(self.thread_id)
            .expect("Could not load active thread")
    }

    pub fn startup() -> crate::Result<()> {
        enable_raw_mode()?;
        execute!(std::io::stderr(), EnterAlternateScreen)?;
        execute!(std::io::stderr(), EnableMouseCapture)?;
        Ok(())
    }

    pub fn shutdown() -> crate::Result<()> {
        execute!(std::io::stderr(), DisableMouseCapture)?;
        execute!(std::io::stderr(), LeaveAlternateScreen)?;

        disable_raw_mode()?;
        Ok(())
    }

    fn max_scroll(&self) -> usize {
        if (self.chat_window_height as usize) >= self.text_len {
            0usize
        } else {
            self.text_len
                .saturating_sub(self.chat_window_height as usize)
                + 2
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

    /// helper function to clear the copy buffer and unset the copy mode state
    fn exit_copy_mode(&mut self) {
        self.copy_select_buf.clear();
        self.copy_mode = false;
    }

    /// 'minor mode' allowing the user to select code block text by its displayed index
    fn update_copy_mode(&mut self, key_event: KeyEvent) -> crate::Result<()> {
        match key_event.code {
            KeyCode::Esc => self.exit_copy_mode(),
            KeyCode::Enter => {
                if let Some(index) = self.selected_block_index {
                    match self.thread().code_blocks().get(index.saturating_sub(1)) {
                        None => {
                            self.bottom_text = Some(format!("No selection for '{}'!", index));
                            self.exit_copy_mode();
                        }
                        Some(block) => {
                            clip::copy(&block.content)?;
                            self.bottom_text =
                                Some(format!("Copied '{}'", string_preview(&block.content, 30)));
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
                            .thread()
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

    fn scroll_up(&mut self, step: usize) {
        self.chat_scroll = self.chat_scroll.saturating_sub(step);
    }

    fn scroll_down(&mut self, step: usize) {
        self.chat_scroll = self
            .chat_scroll
            .saturating_add(step)
            .clamp(0, self.max_scroll());
    }

    fn send_message(&mut self) -> crate::Result<()> {
        let new_message = Message::new_user(&self.user_message);
        self.thread_mut().add_message(new_message);

        // at most 2 listening threads at a time
        // one to stream the tokens, one for meta tasks
        // if we're nearing the token max, request a summary to bring down the context size
        if self.thread().token_use() > 0.7 {
            let summary_rx = self.thread_mut().fetch_summary()?;
            self.summary_rx = Some(summary_rx);
            // if there's still tokens to spare and no title, request a thread title
        } else if self.thread().thread_title().is_none()
            && self.thread().non_sys_messages().len() >= 2
        {
            let title_rx = self.thread_mut().fetch_thread_name()?;
            self.title_rx = Some(title_rx);
        }

        self.reply_rx = Some(stream_thread_reply(self.thread())?);

        self.user_message.clear();

        Ok(())
    }

    fn update_awaiting_send(&mut self) -> crate::Result<()> {
        let input_event = crossterm::event::read()?;

        // key event handling
        if let Event::Key(
            key_event @ KeyEvent {
                kind: event::KeyEventKind::Press,
                code: key_code,
                modifiers: key_modifiers,
                ..
            },
        ) = input_event
        {
            match key_code {
                // ctrl-c to quit
                KeyCode::Char('c') if matches!(key_modifiers, KeyModifiers::CONTROL) => {
                    self.should_quit = true;
                }

                //scroll history up
                KeyCode::Up => self.scroll_up(SCROLL_STEP),

                // scroll history down
                KeyCode::Down => self.scroll_down(SCROLL_STEP),

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
                        self.send_message()?;
                        self.reply_rx = Some(stream_thread_reply(self.thread())?);

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
            // non-keyboard events
        } else {
            match input_event {
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::ScrollUp,
                    ..
                }) => self.scroll_up(SCROLL_STEP * 2),

                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::ScrollDown,
                    ..
                }) => self.scroll_down(SCROLL_STEP * 2),

                _ => (),
            }
        }

        Ok(())
    }

    fn is_recieving(&self) -> bool {
        self.reply_rx.is_some()
    }

    fn update_recieving(&mut self) -> crate::Result<()> {
        self.chat_scroll = self.max_scroll();
        if let Some(rx) = self.reply_rx.as_ref() {
            {
                match rx.recv()? {
                    Some(s) => {
                        self.thread_mut().update(&s);
                    }
                    None => {
                        self.thread_mut().commit_message()?;
                        self.reply_rx = None;
                    }
                }
            }
        }

        Ok(())
    }

    fn update(&mut self) -> crate::Result<()> {
        if let Some(ref summary_rx) = self.summary_rx {
            match summary_rx.try_recv() {
                Ok(s) => {
                    self.thread_mut().add_summary(s);
                    self.summary_rx = None;

                    Ok::<(), crate::Error>(())
                }
                Err(_) if summary_rx.is_empty() => Ok(()),
                Err(e) => Err(e.into()),
            }?;
        }

        if let Some(ref title_rx) = self.title_rx {
            match title_rx.try_recv() {
                Ok(s) => {
                    self.thread_mut().set_title(&s);
                    self.chat_title = Some(s);
                    self.title_rx = None;
                    Ok::<(), crate::Error>(())
                }
                Err(_) if title_rx.is_empty() => Ok(()),
                Err(e) => Err(e.into()),
            }?;
        }

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

    fn ui(&mut self, frame: &mut Frame) -> crate::Result<()> {
        let h_padding = 5u16;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([Constraint::Percentage(80), Constraint::Percentage(20)].as_ref())
            .split(frame.size());

        self.content_line_width = chunks[0].width - (h_padding * 2) - 2;

        let msgs_formatted = self
            .thread()
            .tui_formatted_messages(self.content_line_width);

        let msg_lines = msgs_formatted
            .into_iter()
            .flat_map(|m| m.lines)
            .collect_vec();

        let text_len = msg_lines.len();

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

        let mut scroll_percent = (self.chat_scroll as f64 / self.max_scroll() as f64) * 100.0;

        if scroll_percent.is_nan() {
            scroll_percent = 100.0;
        }

        let chat_title = if let Some(ref title) = self.chat_title {
            string_preview(title, self.content_line_width.saturating_sub(2).into())
        } else {
            "...".into()
        };

        let status_message: Title<'_> = if self.is_recieving() {
            Span::from("[Please Wait]").red().bold().into()
        } else {
            Span::from("[Ready!]").green().into()
        };

        let chat_window_block = Block::default()
            .borders(Borders::ALL)
            .border_type(border_type)
            .border_style(Style::default().fg(border_color))
            .title(string_preview(&chat_title, self.content_line_width.into()).to_string())
            .title(
                Title::from(format!("{:02.0}%", scroll_percent))
                    .alignment(Alignment::Right)
                    .position(Position::Bottom),
            )
            .padding(ratatui::widgets::Padding {
                left: h_padding,
                right: h_padding,
                ..Default::default()
            });

        let chat_window = Paragraph::new(msgs_text).block(chat_window_block);

        frame.render_widget(chat_window, chunks[0]);

        let mut input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(box_color)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .title(
                status_message
                    .position(Position::Bottom)
                    .alignment(Alignment::Right),
            );

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

    pub fn new(session: Session, thread_id: Uuid) -> crate::Result<Self> {
        app_defaults!(session, thread_id)
    }

    pub fn run(&mut self) -> crate::Result<()> {
        set_handler(|| {
            App::shutdown().expect("Cleanup procedure failed");
        })
        .expect("Failed to set handler");

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

    fn show_editor(&mut self, terminal: &mut CrosstermTerminal) -> crate::Result<()> {
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
