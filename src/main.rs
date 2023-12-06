use gpt::app;
use gpt::session::{Role, Session};

async fn _main() -> anyhow::Result<()> {
    app::app_test().await?;

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut session = Session::new_stdout()?;
    session.load_threads()?;

    for thread in session.threads.values() {
        let first_user_msg = thread
            .messages
            .iter()
            .filter(|m| m.role == Role::User)
            .min_by_key(|m| m.timestamp.floor() as usize);

        if let Some(msg) = first_user_msg {
            println!("{} | {}", msg.timestamp, msg.content);
        }
    }

    let thread_id = session.new_thread("You are a helpful assistant")?;

    session.run_shell_stdin(thread_id).await?;

    session.save_to_db()?;

    Ok(())
}
