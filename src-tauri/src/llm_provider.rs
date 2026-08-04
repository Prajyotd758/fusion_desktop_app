// llm_providers.rs
// Handles API calls to Claude (Anthropic) and OpenAI, for use as an alternative/fallback
// to the local llama.cpp server. Uses reqwest (already a dependency).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Claude,
    OpenAI,
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

/// Stored/loaded from your existing config or tauri-plugin-store.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiKeys {
    pub claude_key: Option<String>,
    pub openai_key: Option<String>,
}

// ---------- Claude (Anthropic) ----------

#[derive(Serialize)]
struct ClaudeRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: Vec<ClaudeMessage<'a>>,
}

#[derive(Serialize)]
struct ClaudeMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ClaudeResponse {
    content: Vec<ClaudeContentBlock>,
}

#[derive(Deserialize)]
struct ClaudeContentBlock {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

async fn call_claude(
    client: &reqwest::Client,
    api_key: &str,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, LlmError> {
    let body = ClaudeRequest {
        model: "claude-sonnet-4-6",
        max_tokens: 1024,
        system: system_prompt,
        messages: vec![ClaudeMessage {
            role: "user",
            content: user_prompt,
        }],
    };

    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(LlmError::Api(format!("Claude {status}: {text}")));
    }

    let parsed: ClaudeResponse = resp
        .json()
        .await
        .map_err(|e| LlmError::Parse(e.to_string()))?;

    parsed
        .content
        .into_iter()
        .find(|b| b.kind == "text")
        .and_then(|b| b.text)
        .ok_or_else(|| LlmError::Parse("no text block in Claude response".into()))
}

// ---------- OpenAI ----------

#[derive(Serialize)]
struct OpenAiRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<OpenAiMessage<'a>>,
    response_format: serde_json::Value,
}

#[derive(Serialize)]
struct OpenAiMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiChoiceMessage,
}

#[derive(Deserialize)]
struct OpenAiChoiceMessage {
    content: String,
}

async fn call_openai(
    client: &reqwest::Client,
    api_key: &str,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, LlmError> {
    let resources = crate::resources::get();
    let response_schema = &resources.response_schema;
    let body = OpenAiRequest {
        model: "gpt-4o-mini",
        max_tokens: 1024,
        messages: vec![
            OpenAiMessage {
                role: "system",
                content: system_prompt,
            },
            OpenAiMessage {
                role: "user",
                content: user_prompt,
            },
        ],
        response_format: response_schema.clone(),
    };

    let resp = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(LlmError::Api(format!("OpenAI {status}: {text}")));
    }

    let parsed: OpenAiResponse = resp
        .json()
        .await
        .map_err(|e| LlmError::Parse(e.to_string()))?;

    parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content) // JSON string matching airgrip_response schema; caller does serde_json::from_str
        .ok_or_else(|| LlmError::Parse("no choices in OpenAI response".into()))
}

// ---------- Unified entry point ----------

/// Call whichever cloud provider is requested. Reuse a single reqwest::Client
/// (create it once at app startup / in managed state) rather than per-call.
pub async fn generate_response(
    client: &reqwest::Client,
    provider: Provider,
    keys: &ApiKeys,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, LlmError> {
    match provider {
        Provider::Claude => {
            let key = keys
                .claude_key
                .as_deref()
                .ok_or_else(|| LlmError::Api("Claude API key not set".into()))?;
            call_claude(client, key, system_prompt, user_prompt).await
        }
        Provider::OpenAI => {
            let key = keys
                .openai_key
                .as_deref()
                .ok_or_else(|| LlmError::Api("OpenAI API key not set".into()))?;
            call_openai(client, key, system_prompt, user_prompt).await
        }
    }
}
