use gpt::cli::run_cli;
use gpt::message::{CodeBlock, Message, Role};

fn main() -> anyhow::Result<()> {


    run_cli()?;

    Ok(())
}
