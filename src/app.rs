#![allow(unused)]

use crossterm::event::{
    KeyCode::{self},
    KeyEvent, KeyEventKind, KeyModifiers,
};
use ratatui::{
    layout::{self, Constraint, Layout},
    widgets::Paragraph,
    Frame,
};

use crate::{
    tui::{self, TermEvent},
    Session,
};
use anyhow::anyhow;
use std::{default, marker::PhantomData, ops::Deref};

use crate::{Message, Role, Thread};
use tui_textarea::TextArea;

#[derive(Debug, PartialEq, Eq, Default)]
enum Screen {
    #[default]
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

trait AppMode: Default {
    ///An enum type representing higher-level operations in this mode
    type Action;

    /// Parse a terminal event to a context-appropriate action if possible
    fn parse_event(event: tui::TermEvent) -> anyhow::Result<Option<Self::Action>>;

    /// Function passed to the TUI graphics handler to draw the screen's state
    fn draw_screen(app: &App, frame: &mut Frame);

    /// Alter the app's state according to the action
    fn update(app: &mut App, action: Self::Action);

    fn layout(&self, app: App) -> Layout;

    fn create(app: App) -> Self;
}

#[derive(Debug, Default)]
struct ChatMode<'a> {
    input_box: TextArea<'a>,
}

impl<'a> AppMode for ChatMode<'a> {
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
                    modifiers: KeyModifiers::ALT,
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
        let layout = Layout::default()
            .direction(layout::Direction::Vertical)
            .constraints(vec![Constraint::Percentage(80), Constraint::Percentage(20)])
            .split(frame.size());

        frame.render_widget(Paragraph::new(format!("Chat mode")), frame.size())
    }

    fn update(app: &mut App, action: Self::Action) {
        match action {
            ChatAction::EnterCopyMode => {
                app.screen = Screen::Copy;
            }
            ChatAction::TypeChar('j') => {
                app.count -= 1;
            }

            ChatAction::TypeChar('k') => {
                app.count += 1;
            }

            ChatAction::ExitChat => app.screen = Screen::List,
            _ => {}
        }
    }
    fn layout(&self, app: App) -> Layout {
        Layout::default()
            .direction(layout::Direction::Horizontal)
            .constraints([Constraint::Percentage(10)])
    }

    fn create(app: App) -> Self {
        todo!()
    }
}

#[derive(PartialEq, Eq, Debug, Default)]
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
        frame.render_widget(Paragraph::new("List Mode"), frame.size());
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
    fn layout(&self, app: App) -> Layout {
        todo!()
    }

    fn create(app: App) -> Self {
        todo!()
    }
}

#[derive(PartialEq, Eq, Debug, Default)]
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
        frame.render_widget(Paragraph::new("Copy Mode"), frame.size());
    }

    fn update(app: &mut App, action: Self::Action) {
        match action {
            CopyAction::ExitCopyMode => app.screen = Screen::Chat,
            _ => (),
        }
    }
    fn layout(&self, app: App) -> Layout {
        todo!();
    }

    fn create(app: App) -> Self {
        todo!()
    }
}

#[derive(Default)]
pub struct App {
    screen: Screen,
    should_quit: bool,
    count: i32,
    message_content: String,
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
                match evt {
                    TermEvent::Quit => self.should_quit = true,
                    _ => self.update(evt)?,
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
    let mut app = App::default();
    app.run().await?;
    Ok(())
}
