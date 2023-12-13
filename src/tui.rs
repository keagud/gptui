use crossbeam_channel::{Receiver, Sender};
use itertools::Itertools;

use ratatui::{
    prelude::{Constraint, CrosstermBackend, Direction, Layout, Terminal},
    style::{Color, Style, Stylize},
    text::{Line, Text},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
    Frame,
};
use std::default;
use uuid::Uuid;

use crossterm::{
    event::{
        self,
        Event::{self, Key},
        KeyCode::{self, Char},
        KeyEvent, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

use ctrlc::set_handler;

type SessionHandle = Session<std::io::Sink>;
type ReplyRx = Receiver<Option<String>>;
type CrosstermTerminal = ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stderr>>;

const FPS: f64 = 30.0;

use crate::session::{stream_thread_reply, Message, Role, Session, Thread};

pub struct App {
    should_quit: bool,
    session: SessionHandle,
    thread_id: Option<uuid::Uuid>,
    reply_rx: Option<ReplyRx>,
    incoming_message: String,
    user_message: String,
    tick_duration: std::time::Duration,
    chat_scroll: usize,
}

impl Default for App {
    fn default() -> Self {
        let tick_duration = std::time::Duration::from_secs_f64(1.0 / FPS);
        Self {
            should_quit: false,
            session: Default::default(),
            thread_id: Default::default(),
            reply_rx: Default::default(),
            incoming_message: String::new(),
            user_message: String::new(),
            chat_scroll: 0,
            tick_duration,
        }
    }
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
        if let Event::Key(
            key @ KeyEvent {
                kind: event::KeyEventKind::Press,
                code: key_code,
                modifiers: key_modifiers,
                ..
            },
        ) = crossterm::event::read()?
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

                _ => (),
            }
        }

        Ok(())
    }

    fn is_recieving(&self) -> bool {
        self.reply_rx.is_some()
    }

    fn update_recieving(&mut self) -> anyhow::Result<()> {
        if let Some(chunks) = self.reply_rx.as_ref().map(|rx| rx.try_iter().collect_vec()) {
            for chunk in chunks.into_iter() {
                match chunk {
                    Some(s) => self.incoming_message.push_str(&s),
                    None => {
                        let new_msg = Message::new_asst(&self.incoming_message);
                        self.thread_mut()?.add_message(new_msg);
                        self.incoming_message.clear();

                        if let Some(a) = self.reply_rx.take() {
                            drop(a)
                        }
                        break;
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
                self.update_recieving();
            }

            _ => (),
        }

        Ok(())
    }

    fn ui(&self, frame: &mut Frame) -> anyhow::Result<()> {
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

        let mut msgs_formatted = self.thread()?.tui_formatted_messages()?;

        let mut messages: Vec<String> = self
            .thread()?
            .messages()
            .iter()
            .filter(|m| !m.is_system())
            .map(|msg| {
                let sender = match msg.role {
                    Role::User => "User",
                    Role::Assistant => "Assistant",
                    _ => unreachable!(),
                };

                format!("{}: \n{}\n", sender, msg.content)
            })
            .collect();

        if self.is_recieving() {
            let mut incoming_lines = vec![Line::from(vec![
                Role::Assistant.tui_display_header().unwrap(),
                "\n".into(),
            ])];

            incoming_lines.push(Line::raw(&self.incoming_message));

            msgs_formatted.push(Text::from(incoming_lines));

            messages.push(format!("Assistant: \n{}\n", &self.incoming_message))
        }

        let scroll = chunks[0].height.saturating_sub(self.chat_scroll as u16);

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

        let chat_window = Paragraph::new(msg_lines)
            .block(Block::default().borders(Borders::ALL).title(first_msg))
            .wrap(Wrap { trim: true });

        frame.render_widget(chat_window, chunks[0]);

        let input = Paragraph::new(self.user_message.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(box_color)
                .border_type(ratatui::widgets::BorderType::Thick),
        );

        frame.render_widget(input, chunks[1]);

        Ok(())
    }

    pub fn with_thread(session: SessionHandle, thread_id: Uuid) -> Self {
        Self {
            session,
            thread_id: Some(thread_id),
            ..Default::default()
        }
    }

    pub fn new(prompt: &str) -> anyhow::Result<Self> {
        let mut session = SessionHandle::new_dummy()?;
        let thread_id = Some(session.new_thread(prompt)?);

        Ok(Self {
            session,
            thread_id,
            ..Default::default()
        })
    }

    pub fn run(&mut self) -> anyhow::Result<()> {
        set_handler(|| {
            App::shutdown();
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
