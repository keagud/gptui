use crossterm::{
    cursor,
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        KeyCode, KeyEvent, KeyModifiers,
    },
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::{CrosstermBackend as Backend, Terminal};
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
    task::JoinHandle,
};

use futures::{FutureExt, StreamExt};

use std::ops::{Deref, DerefMut};
use tokio_util::sync::CancellationToken;

pub enum TermEvent {
    CrosstermEvent(crossterm::event::Event),
    Error(Box<dyn std::error::Error + Send + Sync>),
    Tick,
    Render,
    Init,
    Quit,
}

impl TermEvent {
    pub fn raise_err(&self) -> anyhow::Result<()> {
        if let Self::Error(e) = self {
            // TODO this is a bandaid hack
            Err(anyhow::format_err!("{e}"))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug)]
pub struct Tui {
    terminal: Terminal<Backend<std::io::Stderr>>,
    task: JoinHandle<()>,
    event_rx: UnboundedReceiver<TermEvent>,
    event_tx: UnboundedSender<TermEvent>,
    cancellation_token: CancellationToken,
    frame_rate: f64,
    tick_rate: f64,
    mouse: bool,
}

impl Tui {
    pub fn new() -> anyhow::Result<Self> {
        let tick_rate = 4.0;
        let frame_rate = 60.0;

        let terminal = ratatui::Terminal::new(Backend::new(std::io::stderr()))?;

        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let cancellation_token = CancellationToken::new();

        let task = tokio::spawn(async {});

        Ok(Self {
            tick_rate,
            frame_rate,
            task,
            terminal,
            event_tx,
            event_rx,
            cancellation_token,
            mouse: false,
        })
    }

    pub fn tick_delay(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(1.0 / self.tick_rate)
    }

    pub fn render_delay(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(1.0 / self.frame_rate)
    }

    pub fn start(&mut self) {
        let tick_delay = self.tick_delay();
        let render_delay = self.render_delay();

        self.cancel();
        self.cancellation_token = CancellationToken::new();
        let _cancellation_token = self.cancellation_token.clone();

        let _event_tx = self.event_tx.clone();
        self.task = tokio::spawn(async move {
            let mut reader = crossterm::event::EventStream::new();
            let mut tick_interval = tokio::time::interval(tick_delay);
            let mut render_interval = tokio::time::interval(render_delay);

            _event_tx
                .send(TermEvent::Init)
                .expect("Setup of event stream failed");

            loop {
                let tick_delay = tick_interval.tick();
                let render_delay = render_interval.tick();
                let crossterm_event = reader.next().fuse();

                tokio::select! {

                    _ = _cancellation_token.cancelled() => {break;}


                    maybe_event = crossterm_event => {

                        match maybe_event {
                            Some(Ok(evt)) => {
                                // catch ctrl-c at a lower level so it's never missed
                                if let crossterm::event::Event::Key( KeyEvent {
                                    code: KeyCode::Char('c') | KeyCode::Char('d'),
                                    modifiers: KeyModifiers::CONTROL,
                                    ..
                                }) = evt {
                                    _event_tx.send(TermEvent::Quit).unwrap();

                                } else {
                                    // everything else gets delegated to the app logic
                                _event_tx.send(TermEvent::CrosstermEvent(evt)).unwrap();
                                }

                            },
                            Some(Err(e)) => {
                                _event_tx.send(TermEvent::Error(e.into())).unwrap();
                            },

                            None => {}

                        }

                    }


                    _ = tick_delay => {
                        _event_tx.send(TermEvent::Tick).unwrap();


                    }

                    _ = render_delay => {
                        _event_tx.send(TermEvent::Render).unwrap();
                    }

                }
            }
        });
    }

    pub fn stop(&self) -> anyhow::Result<()> {
        self.cancel();

        let mut counter = 0;

        while !self.task.is_finished() {
            std::thread::sleep(std::time::Duration::from_millis(1));
            counter += 1;

            if counter > 50 {
                self.task.abort();
            }

            if counter > 100 {
                eprintln!("Failed to abort task in 100 milliseconds for unknown reason");
                break;
            }
        }

        Ok(())
    }

    pub fn enter(&mut self) -> anyhow::Result<()> {
        crossterm::terminal::enable_raw_mode()?;

        crossterm::execute!(std::io::stderr(), EnterAlternateScreen, cursor::Hide)?;
        crossterm::execute!(std::io::stderr(), EnableBracketedPaste)?;

        if self.mouse {
            crossterm::execute!(std::io::stderr(), EnableMouseCapture)?;
        }

        self.start();

        Ok(())
    }

    pub fn exit(&mut self) -> anyhow::Result<()> {
        self.stop()?;

        if crossterm::terminal::is_raw_mode_enabled()? {
            crossterm::execute!(std::io::stderr(), DisableBracketedPaste)?;

            if self.mouse {
                crossterm::execute!(std::io::stderr(), DisableMouseCapture)?;
            }

            crossterm::execute!(std::io::stderr(), LeaveAlternateScreen, cursor::Show)?;
            crossterm::terminal::disable_raw_mode()?;
        }

        Ok(())
    }

    pub fn cancel(&self) {
        self.cancellation_token.cancel();
    }

    pub fn suspend(&mut self) -> anyhow::Result<()> {
        self.exit()?;

        #[cfg(not(windows))]
        signal_hook::low_level::raise(signal_hook::consts::signal::SIGTSTP)?;

        Ok(())
    }

    pub fn resume(&mut self) -> anyhow::Result<()> {
        self.enter()?;
        Ok(())
    }

    pub async fn next(&mut self) -> Option<TermEvent> {
        self.event_rx.recv().await
    }
}

impl Deref for Tui {
    type Target = ratatui::Terminal<Backend<std::io::Stderr>>;

    fn deref(&self) -> &Self::Target {
        &self.terminal
    }
}

impl DerefMut for Tui {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.terminal
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        self.exit().unwrap();
    }
}

pub fn test_tui() -> anyhow::Result<()> {
    Ok(())
}
