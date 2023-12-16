use gpt::cli::run_cli;
use gpt::message::{CodeBlock, Message, Role};
use gpt::session::Session;
use gpt::tui::App;

fn main() -> anyhow::Result<()> {
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
        let id = session.new_thread("You are a helpful assistant")?;

        session.thread_by_id_mut(id).unwrap().add_message(msg);

        session.thread_by_id(id).unwrap().tui_formatted_messages(70);
    } else {
        run_cli()?;
    }

    Ok(())
}
