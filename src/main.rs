use gptui::cli::run_cli;
use gptui::session::Session;

use gptui::relay;
use itertools::Itertools;

fn main() -> gptui::Result<()> {
    #[cfg(feature = "debug-dump")]
    {
        let mut session = Session::new()?;
        session.load_threads()?;
        session.dump_all();
        println!("Dumped session json");
        std::process::exit(0);
    }

    let args = std::env::args_os().collect_vec();
    if let Some(arg) = args
        .get(1)
        .map(|a| a.to_string_lossy().to_string())
        .filter(|a| a.starts_with("__"))
    {
        match arg.as_str() {
            "__make_config" => {
                gptui::config::Config::write_default().unwrap();
                std::process::exit(0);
            }

            "__relay" => {
                let port = args
                    .get(2)
                    .expect("Port argument is required")
                    .to_str()
                    .expect("Invalid port argument")
                    .to_string();

                return relay::run(&port);
            }

            _ => panic!("Not a valid command: {arg}"),
        }
    } else {
        run_cli()?;
    }

    Ok(())
}
