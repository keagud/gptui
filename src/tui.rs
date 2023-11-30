#![allow(dead_code, unused)]

use crate::Session;
use anyhow;
use crossterm::{
    event::{
        self, Event,
        Event::Key,
        KeyCode::{self, Char},
        KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::{CrosstermBackend, Frame, Terminal},
    widgets::Paragraph,
};

#[derive(Debug, PartialEq, Eq, Default)]
enum Screen {
    Chat,
    #[default]
    List,
}

fn assure_is_press(key_event: KeyEvent) -> Option<KeyEvent> {
    if key_event.kind == KeyEventKind::Press {
        Some(key_event)
    } else {
        None
    }
}

trait FromKeyEvent: Sized {
    fn from_key_event(key: KeyEvent) -> Option<Self>;
}

enum ChatAction {
    TypeChar(char),
    ScrollUp,
    ScrollDown,
    EnterCopyMode,
    ExitChat,
}

enum ListAction {
    EnterChat,
    NewChat,
    SelectionUp,
    SelectionDown,
}

enum CopyAction {
    InputCodeBlockDigit(u8),
    SubmitCopySelection,
    ExitCopyMode,
    ScrollUp,
    ScrollDown,
}

impl FromKeyEvent for ChatAction {
    fn from_key_event(key: KeyEvent) -> Option<Self> {
        use ChatAction::*;

        match assure_is_press(key)? {
            KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::CONTROL,
                ..
            } => Some(ScrollUp),

            KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::CONTROL,
                ..
            } => Some(ScrollDown),

            KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::SHIFT,
                ..
            } => Some(TypeChar(c.to_ascii_uppercase())),

            KeyEvent {
                code: KeyCode::Char(c),
                ..
            } => Some(TypeChar(c)),

            KeyEvent {
                code: KeyCode::Esc, ..
            } => Some(ExitChat),

            KeyEvent {
                code: KeyCode::Char(' '),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => Some(EnterCopyMode),

            _ => None,
        }
    }
}

impl FromKeyEvent for ListAction {
    fn from_key_event(key: KeyEvent) -> Option<Self> {
        match assure_is_press(key)? {
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => Some(ListAction::EnterChat),

            KeyEvent {
                code: KeyCode::Char('n'),
                ..
            } => Some(ListAction::NewChat),

            KeyEvent {
                code: KeyCode::Up, ..
            } => Some(ListAction::SelectionUp),

            KeyEvent {
                code: KeyCode::Down,
                ..
            } => Some(ListAction::SelectionDown),

            _ => None,
        }
    }
}

impl FromKeyEvent for CopyAction {
    fn from_key_event(key: KeyEvent) -> Option<Self> {
        use CopyAction::*;
        match assure_is_press(key)? {
            KeyEvent {
                code: KeyCode::Char(c),
                ..
            } if c.is_digit(10) => c.to_digit(10).map(|d| InputCodeBlockDigit(d as u8)),

            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => Some(SubmitCopySelection),

            KeyEvent {
                code: KeyCode::Esc, ..
            } => Some(ExitCopyMode),

            KeyEvent {
                code: KeyCode::Up, ..
            } => Some(ScrollUp),

            KeyEvent {
                code: KeyCode::Down,
                ..
            } => Some(ScrollDown),

            _ => None,
        }
    }
}

enum Action {
    Tick,
    Quit,
    ChatAction(ChatAction),
    ListAction(ListAction),
    CopyAction(CopyAction),
}

#[derive(Debug, Default)]
pub struct App {
    screen: Screen,
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    fn startup(&self) -> anyhow::Result<()> {
        enable_raw_mode()?;
        execute!(std::io::stderr(), EnterAlternateScreen)?;
        Ok(())
    }

    fn shutdown() -> anyhow::Result<()> {
        execute!(std::io::stderr(), LeaveAlternateScreen)?;
        disable_raw_mode()?;
        Ok(())
    }

    fn get_action(&self, event: Event) -> anyhow::Result<Option<Action>> {
        todo!();
    }

    fn update(&mut self, action: Action) -> anyhow::Result<()> {
        Ok(())
    }

    fn update_chat(&mut self, action: ChatAction) -> anyhow::Result<()> {
        todo!();
    }

    fn update_list(&mut self, action: ListAction) -> anyhow::Result<()> {
        todo!();
    }
}

pub fn test_tui() -> anyhow::Result<()> {
    Ok(())
}
