// llm_providers.rs
use crate::system::helper_functions::groq_key;
use crate::system::r#types::*;

pub async fn call_groq(
    client: &reqwest::Client,
    user_prompt: &str,
    memory_context: &str,
) -> Result<String, LlmError> {
    let resources = crate::resources::get();
    let response_schema = &resources.response_schema;
    let system_prompt = &resources.system_prompt;

    let context = crate::system::helper_functions::build_focus_context();
    let full_user_prompt = if memory_context.is_empty() {
        format!("{context}\n\nUser command: {user_prompt}")
    } else {
        format!("{memory_context}\n\n{context}\n\nUser command: {user_prompt}")
    };

    let body = GroqRequest {
        model: "openai/gpt-oss-120b",
        reasoning_effort: "low",
        max_tokens: 1024,
        temperature: 0.0,
        messages: vec![
            GroqMessage {
                role: "system",
                content: system_prompt,
            },
            GroqMessage {
                role: "user",
                content: &full_user_prompt,
            },
        ],
        response_format: response_schema.clone(),
    };
    let resp = client
        .post("https://api.groq.com/openai/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", groq_key()))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(LlmError::Api(format!("Groq {status}: {text}")));
    }

    let parsed: GroqResponse = resp
        .json()
        .await
        .map_err(|e| LlmError::Parse(e.to_string()))?;

    parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .ok_or_else(|| LlmError::Parse("no choices in Groq response".into()))
}

pub async fn generate_response(
    client: &reqwest::Client,
    provider: Provider,
    user_prompt: &str,
    memory_context: &str,
) -> Result<String, LlmError> {
    match provider {
        Provider::Groq => call_groq(client, user_prompt, memory_context).await,
    }
}

pub async fn query_vision_llm(
    client: &reqwest::Client,
    img_b64: &str,
    question: &str,
    lang: &str,
) -> Result<String, LlmError> {
    const VISION_SYSTEM_PROMPT: &str = "You are a screen-reading assistant. \
        Answer the user's question about the image directly and concisely. \
        Do not show your reasoning, thinking, or analysis process. \
        Do not use <think> tags. Respond with only the final answer, in 2-4 sentences \
        unless the user explicitly asks for more detail.";

    let lang_instruction = if lang == "en" {
        String::new()
    } else {
        format!(" Respond in {lang} (ISO 639-1 code) since the user explicitly asked for that language.")
    };

    let system_prompt = format!("{VISION_SYSTEM_PROMPT}{lang_instruction}");

    let question_no_think = format!("{question} /no_think");

    let body = serde_json::json!({
            "model": "qwen/qwen3.6-27b",
            "max_tokens": 2000,
            "reasoning_effort": "none",
            "messages": [
                { "role": "system", "content": system_prompt },
            {
                "role": "user",
                "content": [
                    { "type": "text", "text": question_no_think },
                    { "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{}", img_b64) }}
                ]
            }
        ]
    });

    let resp = client
        .post("https://api.groq.com/openai/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", groq_key()))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    let raw_text = resp
        .text()
        .await
        .map_err(|e| LlmError::Parse(e.to_string()))?;

    if !status.is_success() {
        return Err(LlmError::Api(format!("Groq vision {status}: {raw_text}")));
    }

    let parsed: GroqResponse =
        serde_json::from_str(&raw_text).map_err(|e| LlmError::Parse(e.to_string()))?;

    let content = parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .ok_or_else(|| LlmError::Parse("no choices in Groq vision response".into()))?;

    Ok(content)
}
