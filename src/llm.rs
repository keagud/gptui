use serde::{Deserialize, Serialize};
use std::fmt::Display;

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Hash,
    strum_macros::EnumVariantNames,
    Serialize,
    Deserialize,
)]
#[repr(u8)]
pub enum LlmModel {
    #[default]
    #[serde(rename = "gpt-4")]
    GPT4,

    #[serde(rename = "gpt-3.5-turbo")]
    GPT35Turbo,
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, Eq, PartialEq)]
pub struct PromptSetting {
    pub label: String,
    pub prompt: String,
    pub model: LlmModel,
    pub color: Option<String>,
}

impl PromptSetting {
    pub fn color(&self) -> Option<&str> {
        self.color.as_deref()
    }
}

impl Default for PromptSetting {
    fn default() -> Self {
        Self {
            label: "Assistant".into(),
            prompt: "You are a helpful assistant".into(),
            color: None,
            model: LlmModel::default(),
        }
    }
}
impl LlmModel {
    pub fn max_context(&self) -> usize {
        match self {
            Self::GPT35Turbo => 4_096,
            Self::GPT4 => 8_192,
        }
    }
}

impl Display for LlmModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let model_label = match self {
            Self::GPT4 => "gpt-4",
            Self::GPT35Turbo => "gpt-3.5-turbo",
        };

        write!(f, "{}", model_label)
    }
}

impl From<LlmModel> for String {
    fn from(val: LlmModel) -> Self {
        val.to_string()
    }
}

impl LlmModel {
    pub fn from_label(label: impl AsRef<str>) -> Option<Self> {
        match label.as_ref() {
            "gpt-4" => Some(Self::GPT4),
            "gpt-3.5-turbo" => Some(Self::GPT35Turbo),
            _ => None,
        }
    }
}
