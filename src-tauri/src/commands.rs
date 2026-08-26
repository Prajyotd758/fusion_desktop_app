use crate::custom_command::CustomCommandsState;
use crate::llm_provider;
use crate::state::LlmChoice;
use crate::state::{self, TaskStatus};
use crate::system;
use crate::system::fast_match;
use crate::system::helper_functions;
use crate::system::r#types::{ApiKeys, Provider};
use crate::tts;
use crate::whisper;
use strsim::jaro_winkler;
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
    keys: &ApiKeys,
    language: &str,
    samples: Vec<f32>,
) -> Result<String, String> {
    let online = crate::ui_callbacks::check_internet().await;

    let text = if language == "en" {
        if online {
            match whisper::transcribe_groq(samples.clone(), language).await {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("[run_transcribe_only] Groq failed, falling back to local: {e}");
                    match whisper::transcribe(app, samples, language).await {
                        Ok(t) => t,
                        Err(e) => {
                            return Err(helper_functions::handle_transcribe_failure(app, e).await)
                        }
                    }
                }
            }
        } else {
            println!("[run_transcribe_only] offline, using local whisper");
            match whisper::transcribe(app, samples, language).await {
                Ok(t) => t,
                Err(e) => return Err(helper_functions::handle_transcribe_failure(app, e).await),
            }
        }
    } else {
        if !online {
            eprintln!("[run_transcribe_only] non-English language '{language}' requires internet, but offline");
            let _ = tts::speak(
                app,
                "No internet connection. Please check your network and try again.",
            );
            state::set_status(TaskStatus::Idle);
            return Err("offline: non-English requires internet".to_string());
        }
        println!("[run_transcribe_only] non-English language '{language}', using Groq");
        match whisper::transcribe_groq(samples, language).await {
            Ok(t) => t,
            Err(e) => return Err(helper_functions::handle_transcribe_failure(app, e).await),
        }
    };

    println!("Transcript: {}", text);

    if !is_valid_transcript(&text) {
        println!("Ignored non-speech/noise transcript: {:?}", text);
        let _ = tts::speak(app, "Sorry, no speech detected. Try again!");
        state::set_status(TaskStatus::Idle);
        return Ok(String::new());
    }

    let custom_state = app.state::<CustomCommandsState>();
    let matched_ops = {
        let commands = custom_state.0.lock().map_err(|e| e.to_string())?;
        let text_normalized = normalize_text(&text);
        println!(
            "[custom_match] checking '{text_normalized}' against {} saved command(s)",
            commands.len()
        );

        let found = commands.iter().find(|c| {
            let keyword_normalized = normalize_text(&c.keyword);
            keyword_normalized == text_normalized
                || jaro_winkler(&keyword_normalized, &text_normalized) >= 0.85
        });

        if let Some(c) = found {
            println!("[custom_match] matched keyword '{}'", c.keyword);
        } else {
            println!("[custom_match] no match found");
        }

        found.map(|c| c.operations.clone())
    };

    if let Some(operations) = matched_ops {
        state::set_status(TaskStatus::Executing);
        let result = crate::system::llm_ops::execute_operations(app, &operations);
        println!("[custom_match] result: {result}");
        state::set_status(TaskStatus::Speaking);
        let _ = tts::speak(app, &result);
        state::set_status(TaskStatus::Idle);
        return Ok(result);
    }

    if let Some((action, arg)) = fast_match::try_fast_match(&text) {
        state::set_status(TaskStatus::Executing);
        match fast_match::execute_action(action, arg.as_deref()) {
            Ok(()) => {
                state::set_status(TaskStatus::Speaking);
                let _ = tts::speak(app, "Done");
                state::set_status(TaskStatus::Idle);
                return Ok("ok".to_string());
            }
            Err(e) => {
                state::set_status(TaskStatus::Speaking);
                let _ = tts::speak(app, "Sorry, that didn't work");
                state::set_status(TaskStatus::Idle);
                return Err(e.to_string());
            }
        }
    }

    // no fast match at all -> offline mode has nothing else to do, return "unsupported"

    // else continue to offline LLM dispatch as before

    // no fast match at all -> offline mode has nothing else to do, return "unsupported"

    let online = crate::ui_callbacks::check_internet().await;

    let llm_result = match llm_choice {
        LlmChoice::None => {
            println!("LLM disabled; no deterministic match for: {:?}", text);
            let _ = tts::speak(app, "Sorry, I didn't understand that command.");
            state::set_status(TaskStatus::Idle);
            return Ok("Sorry, I didn't understand that command.".to_string());
        }
        LlmChoice::Claude | LlmChoice::OpenAi | LlmChoice::Groq => {
            if !online {
                println!("[run_transcribe_only] offline, no fast/custom match, cannot reach LLM");
                let msg = "Sorry, I couldn't find a matching command, and I'm offline right now.";
                let _ = tts::speak(app, msg);
                state::set_status(TaskStatus::Idle);
                return Ok(msg.to_string());
            }

            let resources = crate::resources::get();
            let system_prompt = &resources.system_prompt;
            match llm_provider::generate_response(
                &reqwest::Client::new(),
                Provider::Groq,
                keys,
                &system_prompt,
                &text,
            )
            .await
            {
                Ok(r) => r,
                Err(e) => return Err(helper_functions::handle_transcribe_failure(app, e).await),
            }
        }
    };

    let result = system::handle_llm_response(app, &llm_result);
    println!("execution result:\n{}", result);
    state::set_status(TaskStatus::Idle);

    Ok(text)
}

pub fn normalize_text(s: &str) -> String {
    s.trim().nfc().collect::<String>().to_lowercase()
}
