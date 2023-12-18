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
        let test_message_content = r#"

    Llorum ipsum dolor sit amet:

    ```rust
    pub fn main() {
        println!("Hello, world!");
    }

    ```

Quo usque tandem?


    "#;

        let msg = Message::new_asst(test_message_content);
        dbg!(&msg);

        let mut session = Session::new()?;
        let id = session.new_thread(&Prompt::default())?;

        session.thread_by_id_mut(id).unwrap().add_message(msg);

        session.thread_by_id(id).unwrap().tui_formatted_messages(70);
    } else {
        run_cli()?;
    }

    Ok(())
}
