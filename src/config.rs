use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use toml;

lazy_static::lazy_static! {
    static ref PROJECT_DIRS: directories::ProjectDirs =
    ProjectDirs::from("", "", env!("CARGO_PKG_NAME"))
        .expect("Could not initialize project directories");

    pub static ref CONFIG_DIR: std::path::PathBuf = if cfg!(debug_assertions) {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_assets/config");
        std::fs::create_dir_all(&dir).expect("Failed to create debug config directory");
        dir
    } else {
        PathBuf::from(PROJECT_DIRS.config_dir())
    };


    pub static ref DATA_DIR: std::path::PathBuf = if cfg!(debug_assertions) {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_assets/data");
        std::fs::create_dir_all(&dir).expect("Failed to create debug data directory");
        dir
    } else {
        PathBuf::from(PROJECT_DIRS.data_dir())
    };

}

mod default_config {
    pub(super) const DEFAULT_CONFIG_TOML: &str =
        include_str!(concat!(env!("OUT_DIR"), "/config.toml"));
}

#[derive(Serialize, Deserialize)]
struct Prompt {
    label: String,
    prompt: String,
}

impl Default for Prompt {
    fn default() -> Self {
        Self {
            label: "Assistant".into(),
            prompt: "You are a helpful assistant".into(),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct Config {
    syntax_theme: String,
    editor: Option<String>,
    api_key_var: Option<String>,
    prompts: Vec<Prompt>,
}

impl Default for Config {
    fn default() -> Self {
        toml::from_str(default_config::DEFAULT_CONFIG_TOML)
            .expect("TOML for default config file could not be parsed")
    }
}

impl Config {
    pub fn path(&self) -> PathBuf {
        dbg!(CONFIG_DIR.join("config.toml"))
    }
    fn save(&self) -> anyhow::Result<()> {
        let toml_str = toml::to_string_pretty(self)?;

        fs::write(self.path(), &toml_str)?;

        Ok(())
    }

    pub fn write_default() -> anyhow::Result<()> {
        Config::default().save()
    }
}
