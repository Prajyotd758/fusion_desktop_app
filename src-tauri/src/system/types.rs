use crate::llm_provider::ApiKeys;
use crate::state::LlmChoice;
use serde::Deserialize;
use serde::Serialize;

#[derive(Deserialize)]
pub struct LlmResponse {
    pub intent: String,
    pub response: String,
    #[serde(default)]
    pub operations: Vec<Operation>,
}

#[derive(Deserialize)]
pub struct Operation {
    pub action: String,
    pub parameters: OpParams,
}

#[derive(Deserialize, Default)]
pub struct OpParams {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub parent: String,
    #[serde(default)]
    pub path: String,
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
