use crate::settings::{self, SettingsState};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter};
use crate::system::types::Operation;

/// User's selected backend. `None` = LLM disabled entirely (deterministic matcher only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmChoice {
    Claude,
    OpenAi,
    Groq,
    None,
}

impl Default for LlmChoice {
    fn default() -> Self {
        LlmChoice::Groq
    }
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Idle,
    Recording,
    Transcribing,
    Thinking,      // LLM call in progress
    ScanningImage, // if/when vision step is added
    Executing,     // running the actual system action
    Speaking,      // TTS playback
    Error,
}

static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

pub fn init(app: AppHandle) {
    let _ = APP_HANDLE.set(app);
}

pub fn set_status(status: TaskStatus) {
    if let Some(app) = APP_HANDLE.get() {
        let _ = app.emit("task-status", status);
    }
}

static LAST_OPERATIONS: Mutex<Vec<Operation>> = Mutex::new(Vec::new());

pub fn set_last_operations(ops: Vec<Operation>) {
    println!("[state::set_last_operations] storing {} operation(s)", ops.len());
    if let Ok(mut guard) = LAST_OPERATIONS.lock() {
        *guard = ops;
    } else {
        eprintln!("[state::set_last_operations] failed to lock LAST_OPERATIONS");
    }
}

pub fn take_last_operations() -> Vec<Operation> {
    match LAST_OPERATIONS.lock() {
        Ok(g) => {
            println!("[state::take_last_operations] returning {} operation(s)", g.len());
            g.clone()
        }
        Err(e) => {
            eprintln!("[state::take_last_operations] lock failed: {e}");
            Vec::new()
        }
    }
}