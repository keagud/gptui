#![allow(unused)]

use crossterm::event::{
    KeyCode::{self},
    KeyEvent, KeyEventKind, KeyModifiers,
};
use ratatui::Frame;

use crate::tui;

#[derive(Debug, PartialEq, Eq, Default)]
enum Screen {
    Chat,
    #[default]
    List,
}

pub struct App {
    screen: Screen,
    should_quit: bool,
}

impl App {
    fn ui(&self, frame: &mut Frame) {
        todo!();
    }

    fn update(&mut self, action: AppEvent) -> Option<AppEvent> {
        todo!();
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        let mut tui = tui::Tui::new()?;

        tui.enter()?;

        loop {
            tui.draw(|f| {
                self.ui(f);
            })?;

            if let Some(evt) = tui.next().await {
                let mut maybe_action = parse_event(evt);

                while let Some(action) = maybe_action {
                    maybe_action = self.update(action);
                }
            };

            if self.should_quit {
                break;
            }
        }

        tui.exit()?;

        Ok(())
    }
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

fn parse_event(_event: tui::TermEvent) -> Option<AppEvent> {
    todo!();
}

pub fn app_test() -> anyhow::Result<()> {
    Ok(())
}