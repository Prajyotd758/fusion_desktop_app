use crate::state::LlmChoice;
use serde::Deserialize;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatTurn {
    pub role: String, // "user" | "assistant"
    pub content: String,
    pub timestamp: i64, // unix seconds
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RememberItem {
    pub target: String, // "user_data" | "memory"
    pub content: String,
}

pub struct MemoryState {
    pub user_data: Vec<String>,           // loaded from user_data.md
    pub memory: Vec<String>,              // loaded from memory.md
    pub chat_history: VecDeque<ChatTurn>, // capped, in-memory only
}

pub struct MemoryStateHandle(pub std::sync::Mutex<MemoryState>);

#[derive(Deserialize)]
pub struct LlmResponse {
    pub intent: String,
    pub response: String,
    #[serde(default)]
    pub operations: Vec<Operation>,
    pub response_language: String,
    #[serde(default)]
    pub remember: Vec<RememberItem>,
    #[serde(default)]
    pub needs_clarification: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub action: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub extension: String,
    #[serde(default)]
    pub new_name: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub destination: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub keys: String,
    #[serde(default)]
    pub parameters: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "de_num")]
    pub ms: i64,
    #[serde(default, deserialize_with = "de_num")]
    pub in_secs: i64,
    #[serde(default)]
    pub at: String,
    #[serde(default)]
    pub cron: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub actions: Vec<Operation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppEntry {
    pub exe_path: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AppCache {
    // key = display name from .lnk / registry (lowercase)
    pub apps: HashMap<String, AppEntry>,
}

#[derive(Debug, Deserialize)]
pub struct AliasResult {
    pub exe_path: String,
    pub aliases: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct AliasResponse {
    pub apps: Vec<AliasResult>,
}

impl Operation {
    pub fn normalize(mut self) -> Self {
        if let Some(params) = self.parameters.take() {
            if let Some(obj) = params.as_object() {
                macro_rules! fill {
                    ($field:ident) => {
                        if self.$field.is_empty() {
                            if let Some(v) = obj.get(stringify!($field)).and_then(|v| v.as_str()) {
                                self.$field = v.to_string();
                            }
                        }
                    };
                }
                fill!(name);
                fill!(location);
                fill!(extension);
                fill!(new_name);
                fill!(source);
                fill!(destination);
                fill!(text);
                fill!(keys);
                fill!(label);
                fill!(at);
                fill!(cron);

                let num = |k: &str| {
                    obj.get(k).and_then(|v| {
                        v.as_i64()
                            .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
                    })
                };
                if self.ms == 0 {
                    self.ms = num("ms").unwrap_or(0);
                }
                if self.in_secs == 0 {
                    self.in_secs = num("in_secs").unwrap_or(0);
                }
                if self.actions.is_empty() {
                    if let Some(a) = obj.get("actions") {
                        if let Ok(v) = serde_json::from_value::<Vec<Operation>>(a.clone()) {
                            self.actions = v;
                        }
                    }
                }
            }
        }
        self.actions = self.actions.into_iter().map(Operation::normalize).collect();
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub llm_choice: LlmChoice,

    #[serde(default)]
    pub api_keys: ApiKeys,

    #[serde(default)]
    pub custom_keywords: Vec<String>,

    #[serde(default)]
    pub shortcut: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            llm_choice: LlmChoice::default(),
            api_keys: ApiKeys::default(),
            custom_keywords: Vec::new(),
            shortcut: None,
        }
    }
}

// system/type.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Groq,
}

#[derive(Debug)]
pub enum LlmError {
    Http(reqwest::Error),
    Api(String),
    Parse(String),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Http(e) => write!(f, "HTTP error: {e}"),
            LlmError::Api(e) => write!(f, "API error: {e}"),
            LlmError::Parse(e) => write!(f, "Parse error: {e}"),
        }
    }
}
impl std::error::Error for LlmError {}
impl From<reqwest::Error> for LlmError {
    fn from(e: reqwest::Error) -> Self {
        LlmError::Http(e)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiKeys {
    pub groq_key: Option<String>,
}

#[derive(Serialize)]
pub struct GroqMessage<'a> {
    pub role: &'a str,
    pub content: &'a str,
}

#[derive(Serialize)]
pub struct GroqRequest<'a> {
    pub model: &'a str,
    pub max_tokens: u32,
    pub temperature: f32,
    pub messages: Vec<GroqMessage<'a>>,
    pub response_format: serde_json::Value,
    pub reasoning_effort: &'a str,
}

#[derive(Deserialize)]
pub struct GroqResponse {
    pub choices: Vec<GroqChoice>,
}

#[derive(Deserialize)]
pub struct GroqChoice {
    pub message: GroqChoiceMessage,
}

#[derive(Deserialize)]
pub struct GroqChoiceMessage {
    pub content: String,
}

// // accepts 300, "300", null -> i64 (models sometimes quote numbers)
fn de_num<'de, D: serde::Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    use serde::Deserialize;
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        .unwrap_or(0))
}
