use std::fs::copy;
use std::path::PathBuf;

fn main() {
    #[cfg(feature = "comptime-key")]
    let _ = env!("OPENAI_API_KEY");

    let config_path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/config.toml");
    let dest_path = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("config.toml");

    copy(dbg!(config_path), dbg!(dest_path)).unwrap();
}
