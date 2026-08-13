use crate::settings::{self, SettingsState};
use serde::{Deserialize, Serialize};

/// User's selected backend. `None` = LLM disabled entirely (deterministic matcher only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmChoice {
    Local,
    Claude,
    OpenAi,
    None,
}

impl Default for LlmChoice {
    fn default() -> Self {
        LlmChoice::Local
    }
}

#[tauri::command]
pub fn set_llm_choice(
    app: tauri::AppHandle,
    state: tauri::State<SettingsState>,
    choice: LlmChoice,
) -> Result<(), String> {
    let snapshot = {
        let mut settings = state.0.lock().map_err(|e| e.to_string())?;
        settings.llm_choice = choice;
        settings.clone()
    };
    settings::save(&app, &snapshot)?;
    println!("LLM choice set to: {:?}", choice);
    Ok(())
}

#[tauri::command]
pub fn get_llm_choice(state: tauri::State<SettingsState>) -> Result<LlmChoice, String> {
    let settings = state.0.lock().map_err(|e| e.to_string())?;
    Ok(settings.llm_choice)
}

#[tauri::command]
pub fn get_current_language(state: tauri::State<SettingsState>) -> Result<String, String> {
    let settings = state.0.lock().map_err(|e| e.to_string())?;
    Ok(settings.current_language().to_string())
}

#[tauri::command]
pub fn toggle_language(
    app: tauri::AppHandle,
    state: tauri::State<SettingsState>,
) -> Result<String, String> {
    let snapshot = {
        let mut settings = state.0.lock().map_err(|e| e.to_string())?;
        settings.current_language_index = (settings.current_language_index + 1) % 3;
        settings.clone()
    };
    settings::save(&app, &snapshot)?;
    Ok(snapshot.current_language().to_string())
}

#[tauri::command]
pub fn set_selected_languages(
    app: tauri::AppHandle,
    state: tauri::State<SettingsState>,
    langs: [String; 3],
) -> Result<String, String> {
    let snapshot = {
        let mut settings = state.0.lock().map_err(|e| e.to_string())?;
        settings.selected_languages = langs;
        settings.current_language_index = 0;
        settings.clone()
    };
    settings::save(&app, &snapshot)?;
    Ok(snapshot.current_language().to_string())
}

#[tauri::command]
pub fn set_current_language(
    app: tauri::AppHandle,
    state: tauri::State<SettingsState>,
    lang_code: String,
) -> Result<String, String> {
    let snapshot = {
        let mut settings = state.0.lock().map_err(|e| e.to_string())?;
        match settings
            .selected_languages
            .iter()
            .position(|l| l == &lang_code)
        {
            Some(idx) => {
                settings.current_language_index = idx;
                settings.clone()
            }
            None => {
                return Err(format!(
                    "Language '{}' not in selected languages",
                    lang_code
                ))
            }
        }
    };
    settings::save(&app, &snapshot)?;
    Ok(snapshot.current_language().to_string())
}
