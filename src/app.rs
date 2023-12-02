#![allow(unused)]

use crossterm::event::{
    KeyCode::{self},
    KeyEvent, KeyEventKind, KeyModifiers,
};
use ratatui::{widgets::Paragraph, Frame};

use crate::tui::{self, TermEvent};
use anyhow::anyhow;
use std::{marker::PhantomData, ops::Deref};

#[derive(Debug, PartialEq, Eq)]
enum Screen {
    Chat,
    List,
    Copy,
}

macro_rules! delegate_event {
    ($app:expr, $mode:ty, $event:expr) => {{
        <$mode>::parse_event($event).map(|_maybe_evt| match _maybe_evt {
            None => (),
            Some(_action) => <$mode>::update($app, _action),
        })
    }};
}

macro_rules! delegate_draw {
    ($app:expr, $mode:ty,  $frame:expr ) => {
        <$mode>::draw_screen($app, $frame)
    };
}

macro_rules! placeholder_draw {
    ($app:expr, $frame:expr) => {
        $frame.render_widget(
            Paragraph::new(format!("Counter: {}\n{:?}", $app.count, $app.screen)),
            $frame.size(),
        )
    };
}

trait AppMode {
    ///An enum type representing higher-level operations in this mode
    type Action;

    /// Parse a terminal event to a context-appropriate action if possible
    fn parse_event(event: tui::TermEvent) -> anyhow::Result<Option<Self::Action>>;

    /// Function passed to the TUI graphics handler to draw the screen's state
    fn draw_screen(app: &App, frame: &mut Frame);

    /// Alter the app's state according to the action
    fn update(app: &mut App, action: Self::Action);
}

#[derive(PartialEq, Eq, Debug)]
struct ChatMode();

impl AppMode for ChatMode {
    type Action = ChatAction;
    fn parse_event(event: tui::TermEvent) -> anyhow::Result<Option<Self::Action>> {
        use ChatAction::*;
        event.raise_err()?;

        let result = if let TermEvent::CrosstermEvent(crossterm::event::Event::Key(
            key @ KeyEvent {
                kind: KeyEventKind::Press,
                ..
            },
        )) = event
        {
            match key {
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
        } else {
            None
        };

        Ok(result)
    }

    fn draw_screen(app: &App, frame: &mut Frame) {
        frame.render_widget(
            Paragraph::new(format!("Counter: {}", app.count)),
            frame.size(),
        )
    }

    fn update(app: &mut App, action: Self::Action) {
        match action {
            ChatAction::TypeChar('j') => {
                app.count -= 1;
            }

            ChatAction::TypeChar('k') => {
                app.count += 1;
            }

            ChatAction::ExitChat => app.should_quit = true,
            _ => {}
        }
    }
}

#[derive(PartialEq, Eq, Debug)]
struct ListMode();
impl AppMode for ListMode {
    type Action = ListAction;

    fn update(app: &mut App, action: Self::Action) {
        match action {
            ListAction::EnterChat => {
                app.screen = Screen::Chat;
            }

            _ => (),
        }
    }

    fn draw_screen(app: &App, frame: &mut Frame) {
        placeholder_draw!(app, frame)
    }

    fn parse_event(event: tui::TermEvent) -> anyhow::Result<Option<Self::Action>> {
        event.raise_err()?;

        let result = if let TermEvent::CrosstermEvent(crossterm::event::Event::Key(
            key @ KeyEvent {
                kind: KeyEventKind::Press,
                ..
            },
        )) = event
        {
            match key {
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
        } else {
            None
        };

        Ok(result)
    }
}

#[derive(PartialEq, Eq, Debug)]
struct CopyMode();
impl AppMode for CopyMode {
    type Action = CopyAction;

    fn parse_event(event: tui::TermEvent) -> anyhow::Result<Option<Self::Action>> {
        use CopyAction::*;
        event.raise_err()?;

        let result = if let TermEvent::CrosstermEvent(crossterm::event::Event::Key(
            key @ KeyEvent {
                kind: KeyEventKind::Press,
                ..
            },
        )) = event
        {
            match key {
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
        } else {
            None
        };

        Ok(result)
    }

    fn draw_screen(app: &App, frame: &mut Frame) {
        placeholder_draw!(app, frame);
    }

    fn update(app: &mut App, action: Self::Action) {
        ()
    }
}

pub struct App {
    screen: Screen,
    should_quit: bool,
    count: i32,
}

impl App {
    fn ui(&self, frame: &mut Frame) {
        match self.screen {
            Screen::Chat => delegate_draw!(self, ChatMode, frame),
            Screen::List => delegate_draw!(self, ListMode, frame),
            Screen::Copy => delegate_draw!(self, CopyMode, frame),
        }
    }

    fn update(&mut self, event: TermEvent) -> anyhow::Result<()> {
        match self.screen {
            Screen::Chat => delegate_event!(self, ChatMode, event),
            Screen::List => delegate_event!(self, ListMode, event),
            Screen::Copy => delegate_event!(self, CopyMode, event),
        }
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        let mut tui = tui::Tui::new()?;

        tui.enter()?;

        loop {
            tui.draw(|f| {
                self.ui(f);
            })?;

            if let Some(evt) = tui.next().await {
                self.update(evt)?;
            };

            if self.should_quit {
                break;
            }
        }

        tui.exit()?;

        Ok(())
    }
}

#[derive(Debug)]
enum ChatAction {
    TypeChar(char),
    ScrollUp,
    ScrollDown,
    EnterCopyMode,
    ExitChat,
}

#[derive(Debug)]
enum ListAction {
    EnterChat,
    NewChat,
    SelectionUp,
    SelectionDown,
}

#[derive(Debug)]
enum CopyAction {
    InputCodeBlockDigit(u8),
    SubmitCopySelection,
    ExitCopyMode,
    ScrollUp,
    ScrollDown,
}

#[derive(Debug)]
enum Action {
    Tick,
    Quit,
    Init,
    Render,
    Error(Box<dyn std::error::Error + Send + Sync>),
    ChatAction(ChatAction),
    ListAction(ListAction),
    CopyAction(CopyAction),
}

pub async fn app_test() -> anyhow::Result<()> {
    let mut app = App {
        screen: Screen::Chat,
        should_quit: false,
        count: 0,
    };
    app.run().await?;
    Ok(())
}
