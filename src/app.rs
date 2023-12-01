#![allow(unused)]

use crossterm::event::{
    KeyCode::{self},
    KeyEvent, KeyEventKind, KeyModifiers,
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
                code: KeyCode::Char(' '),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => Some(EnterCopyMode),

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
            } if c.is_ascii_digit() => c.to_digit(10).map(|d| InputCodeBlockDigit(d as u8)),

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

pub enum TermEvent {
    CrosstermEvent(crossterm::event::Event),
    Error(Box<dyn std::error::Error + Send + Sync>),
    Tick,
    Render,
    Init,
}

enum AppEvent {
    Tick,
    Quit,
    Init,
    Render,
    Error(Box<dyn std::error::Error + Send + Sync>),
    ChatAction(ChatAction),
    ListAction(ListAction),
    CopyAction(CopyAction),
}

fn parse_event(_event: crossterm::event::Event) -> Option<AppEvent> {
    todo!();
}
