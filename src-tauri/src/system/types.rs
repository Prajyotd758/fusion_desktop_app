use crate::state::LlmChoice;
use serde::Deserialize;
use serde::Serialize;
use std::collections::VecDeque;

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
}

fn default_response_language() -> String {
    "en".to_string()
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
}

impl Operation {
    /// Pulls values from a stray "parameters" object (if the model nested
    /// them instead of using flat keys) into the flat fields, for any
    /// flat field currently empty.
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
            }
        }
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

    #[serde(default = "default_languages")]
    pub selected_languages: [String; 3],

    #[serde(default)]
    pub current_language_index: usize,
}

fn default_languages() -> [String; 3] {
    ["en".into(), "hi".into(), "mr".into()]
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            llm_choice: LlmChoice::default(),
            api_keys: ApiKeys::default(),
            custom_keywords: Vec::new(),
            shortcut: None,
            selected_languages: default_languages(),
            current_language_index: 0,
        }
    }
}

impl AppSettings {
    pub fn current_language(&self) -> &str {
        &self.selected_languages[self.current_language_index]
    }
}

// system/type.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Claude,
    OpenAI,
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
    pub claude_key: Option<String>,
    pub openai_key: Option<String>,
    pub groq_key: Option<String>,
}

// ---- Claude ----
#[derive(Serialize)]
pub struct ClaudeRequest<'a> {
    pub model: &'a str,
    pub max_tokens: u32,
    pub system: &'a str,
    pub messages: Vec<ClaudeMessage<'a>>,
}

#[derive(Serialize)]
pub struct ClaudeMessage<'a> {
    pub role: &'a str,
    pub content: &'a str,
}

#[derive(Deserialize)]
pub struct ClaudeResponse {
    pub content: Vec<ClaudeContentBlock>,
}

#[derive(Deserialize)]
pub struct ClaudeContentBlock {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: Option<String>,
}

// ---- OpenAI ----
#[derive(Serialize)]
pub struct OpenAiRequest<'a> {
    pub model: &'a str,
    pub max_tokens: u32,
    pub messages: Vec<OpenAiMessage<'a>>,
    pub response_format: serde_json::Value,
}

#[derive(Serialize)]
pub struct OpenAiMessage<'a> {
    pub role: &'a str,
    pub content: &'a str,
}

#[derive(Deserialize)]
pub struct OpenAiResponse {
    pub choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
pub struct OpenAiChoice {
    pub message: OpenAiChoiceMessage,
}

#[derive(Deserialize)]
pub struct OpenAiChoiceMessage {
    pub content: String,
}

// ---- Groq (OpenAI-compatible, reuses OpenAiMessage) ----
#[derive(Serialize)]
pub struct GroqRequest<'a> {
    pub model: &'a str,
    pub max_tokens: u32,
    pub temperature: f32,
    pub messages: Vec<OpenAiMessage<'a>>,
    pub response_format: serde_json::Value,
    pub reasoning_effort: &'a str,
}

#[derive(Deserialize)]
pub struct GroqResponse {
    pub choices: Vec<OpenAiChoice>,
}
