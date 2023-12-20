use gpt::cli::run_cli;
use gpt::message::Message;
use gpt::session::Session;

#[cfg(debug_assertions)]

fn main() -> anyhow::Result<()> {
    use gpt::config::Prompt;

    #[cfg(debug_assertions)]
    {
        if let Some(arg) = std::env::args_os()
            .nth(1)
            .map(|a| a.to_string_lossy().to_string())
            .filter(|a| a.starts_with("__"))
        {
            match arg.as_str() {
                "__make_config" => {
                    gpt::config::Config::write_default().unwrap();
                }

                _ => (),
            }

            std::process::exit(0);
        }
    }

    if std::env::var("TEST").is_ok_and(|v| v == "1") {
        let mut session = Session::new()?;
        session.load_threads()?;
        let ordered = session.ordered_threads();
        let (_, thread) = ordered.last().unwrap();
        let title = thread.fetch_thread_name()?;
        dbg!(&title);
    } else {
        run_cli()?;
    }

    Ok(())
}
