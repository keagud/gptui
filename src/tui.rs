use std::sync::mpsc::Receiver;

use ratatui::{
    prelude::{CrosstermBackend, Terminal},
    widgets::Padding, Frame,
};

use crossterm::{
    event::{self, Event::Key, KeyCode::Char},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

type SessionHandle = Session<std::io::Sink>;

type ReplyRx = Receiver<Option<String>>;

use crate::session::{stream_user_message, Message, Role, Session, Thread};

enum AppState {
    AwaitingSend,
    AwaitingReply,
    Recieving,
}

struct App {
    should_quit: bool,
    session: SessionHandle,
    thread_id: Option<uuid::Uuid>,
    reply_rx: Option<ReplyRx>,
}

impl App {
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

    fn update(&mut self) -> anyhow::Result<()> {
        Ok(())
    }


    fn ui(&self, frame: &mut Frame) -> anyhow::Result<()> {Ok(())}


}
