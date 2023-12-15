use crossbeam_channel::Receiver;
use itertools::Itertools;

use ratatui::{
    prelude::{Constraint, CrosstermBackend, Direction, Layout},
    style::{Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

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

type ReplyRx = Receiver<Option<String>>;

type Backend = ratatui::backend::CrosstermBackend<std::io::Stderr>;
type CrosstermTerminal = ratatui::Terminal<Backend>;

const FPS: f64 = 30.0;
const SCROLL_STEP: usize = 1;

use crate::session::{stream_thread_reply, Message, Role, Session, Thread};

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
    incoming_message: String,
    user_message: String,
    tick_duration: std::time::Duration,
    chat_scroll: usize,
}

macro_rules! app_defaults {
    ($session:expr, $thread_id:expr) => {{
        let tick_duration = std::time::Duration::from_secs_f64(1.0 / FPS);
        Self {
            should_quit: false,
            session: $session,
            thread_id: Some($thread_id),
            reply_rx: Default::default(),
            incoming_message: String::new(),
            user_message: String::new(),
            chat_scroll: 0,
            tick_duration,
        }
    }};

    ($session:expr) => {{
        let tick_duration = std::time::Duration::from_secs_f64(1.0 / FPS);
        Self {
            should_quit: false,
            session: $session,
            thread_id: None,
            reply_rx: Default::default(),
            incoming_message: String::new(),
            user_message: String::new(),
            chat_scroll: 0,
            tick_duration,
        }
    }};
}

macro_rules! thread_missing {
    ($opt:expr) => {
        $opt.ok_or_else(|| anyhow::format_err!("Not connected to a thread"))
    };
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

    fn update_awaiting_send(&mut self) -> anyhow::Result<()> {
        if let Event::Key(KeyEvent {
            kind: event::KeyEventKind::Press,
            code: key_code,
            modifiers: key_modifiers,
            ..
        }) = crossterm::event::read()?
        {
            match key_code {
                KeyCode::Char('c') if matches!(key_modifiers, KeyModifiers::CONTROL) => {
                    self.should_quit = true;
                }

                KeyCode::Char(c) if matches!(key_modifiers, KeyModifiers::SHIFT) => {
                    self.user_message.push(c.to_ascii_uppercase());
                }

                KeyCode::Char(c) => {
                    self.user_message.push(c);
                }

                KeyCode::Backspace => {
                    self.user_message.pop();
                }

                KeyCode::Enter if matches!(key_modifiers, KeyModifiers::CONTROL) => {
                    self.user_message.push('\n');
                }

                KeyCode::Enter => {
                    if !self.user_message.is_empty() {
                        let new_message = Message::new_user(&self.user_message);
                        self.thread_mut()?.add_message(new_message);

                        self.reply_rx = Some(stream_thread_reply(self.thread()?)?);

                        self.user_message.clear();
                    }
                }

                KeyCode::Up => {
                    self.chat_scroll = self.chat_scroll.saturating_sub(SCROLL_STEP);
                }

                KeyCode::Down => {
                    self.chat_scroll = self.chat_scroll.saturating_add(SCROLL_STEP);
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
        let v_padding = 5u16;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([Constraint::Percentage(80), Constraint::Percentage(20)].as_ref())
            .split(frame.size());

        let first_msg = self
            .thread()?
            .first_message()
            .map(|m| m.content.as_str())
            .unwrap_or("");

        let mut msgs_formatted = self
            .thread()?
            .tui_formatted_messages(chunks[0].width - (v_padding * 2) - 2 )?;

        if self.is_recieving() {
            let mut incoming_lines = vec![Line::from(vec![
                Role::Assistant.tui_display_header(),
                "\n".into(),
            ])];

            incoming_lines.extend(self.incoming_message.lines().map(|ln| Line::from(ln)));

            msgs_formatted.push(Text::from(incoming_lines));
        }

        let box_color = if self.is_recieving() {
            Style::new().white()
        } else {
            Style::new().blue()
        };

        let msg_lines = msgs_formatted
            .into_iter()
            .map(|m| m.lines)
            .flatten()
            .collect_vec();

        let msgs_text = Text::from(msg_lines);

        let window_height = chunks[0].height as usize;

        let scroll: u16 = if window_height > msgs_text.height() {
            // if window is larger than text, no need to scroll
            0usize
        } else {
            self.chat_scroll
        } as u16;

        let chat_window = Paragraph::new(msgs_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(first_msg)
                    .padding(ratatui::widgets::Padding {
                        left: v_padding,
                        right: v_padding,
                        ..Default::default()
                    }),
            )
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0));

        frame.render_widget(chat_window, chunks[0]);

        let input = Paragraph::new(self.user_message.as_str())
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(box_color)
                    .border_type(ratatui::widgets::BorderType::Thick),
            );

        frame.render_widget(input, chunks[1]);

        Ok(())
    }

    pub fn with_thread(session: Session, thread_id: Uuid) -> Self {
        app_defaults!(session, thread_id)
    }

    pub fn new(prompt: &str) -> anyhow::Result<Self> {
        let mut session = Session::new()?;

        Ok(app_defaults!(session))
    }

    pub fn run(&mut self) -> anyhow::Result<()> {
        set_handler(|| {
            App::shutdown().expect("Cleanup procedure failed");
        })?;

        let backend = CrosstermBackend::new(std::io::stderr());

        let mut terminal = CrosstermTerminal::new(backend)?;
        App::startup()?;

        while !self.should_quit {
            self.update()?;

            terminal.draw(|frame| self.ui(frame).unwrap())?;
        }

        App::shutdown()?;
        Ok(())
    }
}
