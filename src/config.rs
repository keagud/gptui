use anyhow::format_err;
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

    static ref CONFIG_DIR: std::path::PathBuf = if cfg!(debug_assertions) {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_assets/config");
        std::fs::create_dir_all(&dir).expect("Failed to create debug config directory");
        dir
    } else {
        PathBuf::from(PROJECT_DIRS.config_dir())
    };


    static ref DATA_DIR: std::path::PathBuf = if cfg!(debug_assertions) {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_assets/data");
        std::fs::create_dir_all(&dir).expect("Failed to create debug data directory");
        dir
    } else {
        PathBuf::from(PROJECT_DIRS.data_dir())
    };

    pub static ref CONFIG: Config = Config::load().expect("Failed to load config file");

}

mod default_config {
    // This is so the initial config file can contain explanatory comments
    pub(super) const DEFAULT_CONFIG_TOML: &str =
        include_str!(concat!(env!("OUT_DIR"), "/config.toml"));
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Prompt {
    label: String,
    prompt: String,
    color: Option<String>,
}

impl Prompt {
    pub fn label(&self) -> &str {
        self.label.as_str()
    }

    pub fn prompt(&self) -> &str {
        self.prompt.as_str()
    }

    pub fn color(&self) -> Option<&str> {
        self.color.as_deref()
    }
}

impl Default for Prompt {
    fn default() -> Self {
        Self {
            label: "Assistant".into(),
            prompt: "You are a helpful assistant".into(),
            color: None,
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
    pub fn prompts(&self) -> Vec<&Prompt> {
        self.prompts.iter().collect()
    }

    pub fn get_prompt(&self, label: &str) -> Option<&Prompt> {

        self.prompts.iter().find(|p| p.label == label)


    }
    pub fn data_dir(&self) -> &'static PathBuf {
        &DATA_DIR
    }

    pub fn config_dir(&self) -> &'static PathBuf {
        &CONFIG_DIR
    }

    #[cfg(feature = "comptime-key")]
    pub fn api_key(&self) -> String {
        std::env!("OPENAI_API_KEY").into()
    }

    #[cfg(not(feature = "comptime-key"))]
    pub fn api_key(&self) -> String {
        let key_varname = self.api_key_var.as_deref().unwrap_or("OPENAI_API_KEY");

        std::env::var_os(key_varname)
            .map(|s| s.to_string_lossy().to_string())
            .expect("No API key was found in the environment")
            .into()
    }

    pub fn load() -> anyhow::Result<Self> {
        let loaded_config = if !Self::path().try_exists()? {
            // If no config present, save the default one
            std::fs::write(Self::path(), default_config::DEFAULT_CONFIG_TOML)?;
            Self::default()
        } else {
            let loaded_config_str = fs::read_to_string(Self::path())?;
            toml::from_str(&loaded_config_str)?
        };

        // panics if api key is not present
        let _ = loaded_config.api_key();

        Ok(loaded_config)
    }

    pub fn path() -> PathBuf {
        CONFIG_DIR.join("config.toml")
    }
    fn save(&self) -> anyhow::Result<()> {
        let toml_str = toml::to_string_pretty(self)?;

        fs::write(Self::path(), &toml_str)?;

        Ok(())
    }

    pub fn write_default() -> anyhow::Result<()> {
        Config::default().save()
    }
}
