use crate::custom_command::CustomCommandsState;
use crate::llm_provider;
use crate::state::LlmChoice;
use crate::state::{self, TaskStatus};
use crate::system;
use crate::system::fast_match;
use crate::system::helper_functions;
use crate::system::r#types::Provider;
use crate::tts;
use crate::whisper;
use tauri::Manager;
use unicode_normalization::UnicodeNormalization;

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
    app: &tauri::AppHandle,
    llm_choice: LlmChoice,
    samples: Vec<f32>,
) -> Result<String, String> {
    // Try Groq STT first; on any failure (offline, timeout, API error) fall back to local whisper.
    let text = match whisper::transcribe_groq(samples.clone()).await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("[run_transcribe_only] Groq STT failed, using local whisper: {e}");
            match whisper::transcribe(app, samples).await {
                Ok(t) => t,
                Err(e) => return Err(helper_functions::handle_transcribe_failure(app, e).await),
            }
        }
    };

    println!("Transcript: {}", text);

    if !is_valid_transcript(&text) {
        println!("Ignored non-speech/noise transcript: {:?}", text);
        let _ = tts::speak(app, "Sorry, no speech detected. Try again!", "en");
        state::set_status(TaskStatus::Idle);
        return Ok(String::new());
    }

    // 1. Custom (user-saved) commands
    let custom_state = app.state::<CustomCommandsState>();
    let matched_ops = {
        let commands = custom_state.0.lock().map_err(|e| e.to_string())?;
        let text_normalized = normalize_text(&text);
        println!(
            "[custom_match] checking '{text_normalized}' against {} saved command(s)",
            commands.len()
        );

        let found = crate::custom_command::find_matching_command(&commands, &text_normalized);

        if let Some(c) = found {
            println!("[custom_match] matched keyword '{}'", c.keyword);
        } else {
            println!("[custom_match] no match found");
        }

        found.map(|c| c.operations.clone())
    };

    if let Some(operations) = matched_ops {
        state::set_status(TaskStatus::Executing);
        let result = crate::system::llm_ops::execute_operations(app, &operations, "en", "Done");
        state::set_status(TaskStatus::Speaking);
        let _ = tts::speak(app, &result, "en");
        state::set_status(TaskStatus::Idle);
        return Ok(result);
    }

    // 2. Deterministic fast-match
    if let Some((action, arg)) = fast_match::try_fast_match(&text) {
        state::set_status(TaskStatus::Executing);
        match fast_match::execute_action(action, arg.as_deref()) {
            Ok(()) => {
                state::set_status(TaskStatus::Speaking);
                let _ = tts::speak(app, "Done", "en");
                state::set_status(TaskStatus::Idle);
                return Ok("ok".to_string());
            }
            Err(e) => {
                state::set_status(TaskStatus::Speaking);
                let _ = tts::speak(app, "Sorry, that didn't work", "en");
                state::set_status(TaskStatus::Idle);
                return Err(e.to_string());
            }
        }
    }

    // 3. LLM dispatch
    let llm_result = match llm_choice {
        LlmChoice::None => {
            println!("LLM disabled; no deterministic match for: {:?}", text);
            let msg = "Sorry, I didn't understand that command.";
            let _ = tts::speak(app, msg, "en");
            state::set_status(TaskStatus::Idle);
            return Ok(msg.to_string());
        }
        LlmChoice::Groq => {
            let memory_state = app.state::<crate::system::types::MemoryStateHandle>();
            let memory_context = crate::system::memory::build_memory_context(&memory_state.0);
            crate::system::memory::push_chat_turn(&memory_state.0, "user", &text);

            match llm_provider::generate_response(
                helper_functions::http_client(),
                Provider::Groq,
                &text,
                &memory_context,
            )
            .await
            {
                Ok(r) => r,
                // Only report offline when the request genuinely failed to connect.
                Err(crate::system::types::LlmError::Http(e))
                    if e.is_connect() || e.is_timeout() =>
                {
                    eprintln!("[run_transcribe_only] LLM unreachable: {e}");
                    let msg = "Sorry, I couldn't find a matching command, and I can't reach the internet right now.";
                    let _ = tts::speak(app, msg, "en");
                    state::set_status(TaskStatus::Idle);
                    return Ok(msg.to_string());
                }
                Err(e) => return Err(helper_functions::handle_transcribe_failure(app, e).await),
            }
        }
    };

    // speak() happens inside handle_llm_response, using response_language from the LLM
    let result = system::handle_llm_response(app, &llm_result);
    println!("execution result:\n{}", result);
    state::set_status(TaskStatus::Idle);

    Ok(text)
}

pub fn normalize_text(s: &str) -> String {
    s.trim().nfc().collect::<String>().to_lowercase()
}
