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

/// Holds the 3 user-selected language codes and which one is currently active.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageState {
    pub selected_languages: [String; 3], // e.g. ["en", "hi", "mr"]
    pub current_index: usize,
}

impl Default for LanguageState {
    fn default() -> Self {
        Self {
            selected_languages: ["en".into(), "hi".into(), "mr".into()],
            current_index: 0,
        }
    }
}

impl LanguageState {
    pub fn current(&self) -> &str {
        &self.selected_languages[self.current_index]
    }

    pub fn toggle(&mut self) -> &str {
        self.current_index = (self.current_index + 1) % 3;
        self.current()
    }

    pub fn set_languages(&mut self, langs: [String; 3]) {
        self.selected_languages = langs;
        self.current_index = 0;
    }

    pub fn set_current(&mut self, lang_code: &str) -> bool {
        if let Some(idx) = self.selected_languages.iter().position(|l| l == lang_code) {
            self.current_index = idx;
            true
        } else {
            false
        }
    }
}

#[tauri::command]
pub fn toggle_language(state: tauri::State<std::sync::Mutex<LanguageState>>) -> String {
    let mut lang = state.lock().unwrap();
    lang.toggle().to_string()
}

#[tauri::command]
pub fn get_current_language(state: tauri::State<std::sync::Mutex<LanguageState>>) -> String {
    state.lock().unwrap().current().to_string()
}

#[tauri::command]
pub fn set_selected_languages(
    state: tauri::State<std::sync::Mutex<LanguageState>>,
    langs: [String; 3],
) -> String {
    let mut lang = state.lock().unwrap();
    lang.set_languages(langs);
    lang.current().to_string()
}

#[tauri::command]
pub fn set_current_language(
    state: tauri::State<std::sync::Mutex<LanguageState>>,
    lang_code: String,
) -> Result<String, String> {
    let mut lang = state.lock().unwrap();
    if lang.set_current(&lang_code) {
        Ok(lang.current().to_string())
    } else {
        Err(format!(
            "Language '{}' not in selected languages",
            lang_code
        ))
    }
}
