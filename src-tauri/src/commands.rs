use crate::llm_provider::{ApiKeys, Provider};
use crate::state::LlmChoice;
use crate::system;
use crate::whisper;
use crate::{llm, llm_provider};

pub fn is_valid_transcript(text: &str) -> bool {
    let trimmed = text.trim();

    if trimmed.is_empty() {
        return false;
    }

    let noise_markers = [
        "[BLANK_AUDIO]",
        "[SILENCE]",
        "(SILENCE)",
        "[NOISE]",
        "[MUSIC]",
        "...",
    ];
    let upper = trimmed.to_uppercase();
    if noise_markers
        .iter()
        .any(|m| upper == *m || upper.contains(m))
    {
        return false;
    }

    if !trimmed.chars().any(|c| c.is_alphanumeric()) {
        return false;
    }

    if trimmed.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
        return false;
    }

    true
}

pub async fn run_transcribe_only(
    llm_choice: LlmChoice,
    keys: &ApiKeys,
    language: &str,
) -> Result<String, String> {
    let text = whisper::transcribe(language)
        .await
        .map_err(|e| e.to_string())?;
    println!("Transcript: {}", text);

    if !is_valid_transcript(&text) {
        println!("Ignored non-speech/noise transcript: {:?}", text);
        return Ok(String::new());
    }

    if let Some(result) = system::execute(&text) {
        return Ok(result);
    }

    let llm_result = match llm_choice {
        LlmChoice::None => {
            println!("LLM disabled; no deterministic match for: {:?}", text);
            return Ok("Sorry, I didn't understand that command.".to_string());
        }
        LlmChoice::Local => llm::interpret_command(&text)
            .await
            .map_err(|e| e.to_string())?,
        LlmChoice::Claude | LlmChoice::OpenAi => {
            let provider = match llm_choice {
                LlmChoice::Claude => Provider::Claude,
                LlmChoice::OpenAi => Provider::OpenAI,
                _ => unreachable!(),
            };
            let resources = crate::resources::get();
            let system_prompt = &resources.system_prompt;
            llm_provider::generate_response(
                &reqwest::Client::new(),
                provider,
                keys,
                &system_prompt,
                &text,
            )
            .await
            .map_err(|e| e.to_string())?
        }
    };

    println!("LLM:\n{}", llm_result);
    let result = system::handle_llm_response(&llm_result);
    println!("execution result:\n{}", result);

    Ok(result)
}
