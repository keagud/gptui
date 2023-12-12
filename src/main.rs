use std::io::{self, Read, Write};

use crossterm::QueueableCommand;
use ctrlc::set_handler;
use gpt::cli::run_cli;
use gpt::session::{stream_thread_reply, Message, Session, Thread};
use gpt::tui::App;

fn main() -> anyhow::Result<()> {
    App::run()?;

    // let mut session = Session::new_dummy()?;
    // let thread_id = session.new_thread("You are a helpful assistant")?;

    // session
    //     .thread_by_id_mut(thread_id)
    //     .unwrap()
    //     .add_message(Message::new_user(
    //         "Are there any warm-blooded animals that aren't mammals or birds?",
    //     ));

    // let rx = stream_thread_reply(session.thread_by_id(thread_id).unwrap())?;

    // let mut stdout = std::io::stdout();

    // while let Some(reply) = rx.recv()? {
    //     stdout.write_all(reply.as_bytes())?;
    //     stdout.flush()?;
    // }

    Ok(())
}
