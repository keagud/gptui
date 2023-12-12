use std::{default, sync::mpsc::Receiver};

use ratatui::{
    prelude::{Constraint, CrosstermBackend, Direction, Layout, Terminal},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
    Frame,
};

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

type SessionHandle = Session<std::io::Sink>;
type ReplyRx = Receiver<Option<String>>;
type CrosstermTerminal = ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stderr>>;

const FPS: f64 = 30f64;

use crate::session::{stream_thread_reply, Message, Role, Session, Thread};

enum AppState {
    AwaitingSend,
    AwaitingReply,
    Recieving,
}

pub struct App {
    should_quit: bool,
    session: SessionHandle,
    thread_id: Option<uuid::Uuid>,
    state: AppState,
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
            state: AppState::AwaitingSend,
            incoming_message: String::new(),
            user_message: String::new(),
            chat_scroll: 0,
            tick_duration,
        }
    }
}

impl App {
    fn new() -> Self {
        Self::default()
    }

    fn messages(&self) -> Option<Vec<&Message>> {
        self.thread().map(|t| t.messages())
    }
    fn thread(&self) -> Option<&Thread> {
        self.thread_id.and_then(|id| self.session.thread_by_id(id))
    }

    fn thread_mut(&mut self) -> Option<&mut Thread> {
        self.thread_id
            .and_then(|id| self.session.thread_by_id_mut(id))
    }

    fn startup() -> anyhow::Result<()> {
        enable_raw_mode()?;
        execute!(std::io::stderr(), EnterAlternateScreen)?;
        Ok(())
    }

    fn shutdown() -> anyhow::Result<()> {
        execute!(std::io::stderr(), LeaveAlternateScreen)?;
        disable_raw_mode()?;
        Ok(())
    }

    fn update_awaiting_send(&mut self) -> anyhow::Result<()> {
        if let Event::Key(
            key @ KeyEvent {
                kind: event::KeyEventKind::Press,
                code: key_code @ _,
                modifiers: key_modifiers @ _,
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
                        self.thread_mut().unwrap().add_message(new_message);

                        let rx = stream_thread_reply(&self.thread().unwrap())?;
                        self.reply_rx = Some(rx);

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
        if let Some(ref rx) = self.reply_rx {
            match rx.recv()? {
                Some(chunk) => {
                    self.incoming_message.push_str(&chunk);
                }
                None => {
                    self.reply_rx = None;

                    let new_msg = Message::new_asst(&self.incoming_message);
                    self.thread_mut().unwrap().add_message(new_msg);

                    self.incoming_message.clear();
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

    fn ui(&self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([Constraint::Percentage(80), Constraint::Percentage(20)].as_ref())
            .split(frame.size());

        let mut messages: Vec<String> = self
            .messages()
            .unwrap_or_default()
            .iter()
            .filter(|m| !m.is_system())
            .map(|msg| {
                let sender = match msg.role {
                    Role::User => "User",
                    Role::Assistant => "Assistant",
                    _ => unreachable!(),
                };

                format!("{}: {}\n", sender, msg.content)
            })
            .collect();

        if self.is_recieving() {
            messages.push(format!("Assistant: {}\n", &self.incoming_message))
        }

        let scroll = chunks[0].height.saturating_sub(self.chat_scroll as u16);

        let f = self
            .thread_id
            .and_then(|id| self.session.thread_by_id(id))
            .and_then(|t| t.first_message())
            .map(|m| m.content.to_owned())
            .unwrap_or_default();

        let chat_window = Paragraph::new(messages.join("\n"))
            .block(Block::default().borders(Borders::ALL).title(f))
            .wrap(Wrap { trim: true })
            .scroll((scroll, 0));

        frame.render_widget(chat_window, chunks[0]);

        let input = Paragraph::new(self.user_message.as_str())
            .block(Block::default().borders(Borders::ALL).title("Input"));

        frame.render_widget(input, chunks[1]);
    }

    pub fn run() -> anyhow::Result<()> {
        let mut app = App::new();

        let backend = CrosstermBackend::new(std::io::stderr());

        let mut terminal = CrosstermTerminal::new(backend)?;
        App::startup()?;

        while !app.should_quit {
            app.update()?;
            terminal.draw(|frame| app.ui(frame))?;
        }

        App::shutdown()?;
        Ok(())
    }
}
