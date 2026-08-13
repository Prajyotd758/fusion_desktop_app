use crate::system::helper_functions::get_focused_app;
use anyhow::{anyhow, Result};
use reqwest::Client;
use serde_json::{json, Value};

pub async fn interpret_command(transcript: &str) -> Result<String> {
    let client = Client::new();

    let context_line = match get_focused_app() {
        Some((process, title)) => format!(
            "Currently focused app: {process} (\"{title}\"). If the user's request needs an app-specific keyboard shortcut, use your own knowledge of that app's shortcuts and issue it via 'press_keys' with the raw combo (e.g. ctrl+`, ctrl+shift+p)."
        ),
        None => "No focused app detected".to_string(),
    };

    let full_prompt = format!("{context_line}\n\nUser command: {transcript}");

    // Get cached resources (loaded once during app startup)
    let resources = crate::resources::get();

    let system_prompt = &resources.system_prompt;
    let response_schema = &resources.response_schema;

    let response = client
        .post("http://127.0.0.1:8081/v1/chat/completions")
        .json(&json!({
            "model": "qwen2.5",

            "messages": [
                {
                    "role": "system",
                    "content": system_prompt
                },
                {
                    "role": "user",
                    "content": full_prompt
                }
            ],

            "temperature": 0.0,
            "top_p": 1.0,
            "stream": false,

            "response_format": response_schema
        }))
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(anyhow!("LLM returned {}", response.status()));
    }

    let json: Value = response.json().await?;

    let content = json["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| anyhow!("LLM returned empty content"))?
        .to_string();

    Ok(content)
}
