use std::fs::copy;
use std::path::PathBuf;

fn copy_asset_file(filename: &str) {
    let asset_path = format!("{}/assets/{}", env!("CARGO_MANIFEST_DIR"), filename);
    let dest_path = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join(filename);

    copy(asset_path, dest_path).expect("Failed to set up build assets");
}

fn main() {
    #[cfg(feature = "comptime-key")]
    let _ = env!("OPENAI_API_KEY");

    copy_asset_file("config.toml");
    copy_asset_file("init.sql");
}
