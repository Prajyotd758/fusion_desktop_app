use crate::commands::normalize_text;
use crate::system::types::Operation;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use strsim::jaro_winkler;
use tauri::Manager;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserCommand {
    pub keyword: String,
    pub operations: Vec<Operation>,
}

pub struct CustomCommandsState(pub Mutex<Vec<UserCommand>>);

fn file_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("custom_commands.json"))
}

pub fn load(app: &tauri::AppHandle) -> Vec<UserCommand> {
    let path = match file_path(app) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[custom_commands::load] path error: {e}");
            return Vec::new();
        }
    };

    if !path.exists() {
        return Vec::new();
    }

    match fs::read_to_string(&path) {
        Ok(raw) => match serde_json::from_str::<Vec<UserCommand>>(&raw) {
            Ok(commands) => commands,
            Err(_e) => Vec::new(),
        },
        Err(_e) => Vec::new(),
    }
}

pub fn save(app: &tauri::AppHandle, commands: &[UserCommand]) -> Result<(), String> {
    let path = file_path(app)?;
    let raw = serde_json::to_string_pretty(commands).map_err(|e| e.to_string())?;
    println!(
        "[custom_commands::save] writing {} command(s) to {:?}",
        commands.len(),
        path
    );
    match fs::write(&path, raw) {
        Ok(()) => {
            println!("[custom_commands::save] write succeeded");
            Ok(())
        }
        Err(e) => {
            eprintln!("[custom_commands::save] write failed: {e}");
            Err(e.to_string())
        }
    }
}

/// Finds the best custom-command match for the given transcript, or None.
/// Guards against the false-positive matching strsim alone allows:
/// - very short keywords are excluded from fuzzy matching entirely (a 3-4
///   char keyword can accidentally score >0.85 against almost anything)
/// - keyword and transcript must be reasonably close in length
/// - similarity threshold raised from 0.85 to 0.92
pub fn find_matching_command<'a>(
    commands: &'a [UserCommand],
    text_normalized: &str,
) -> Option<&'a UserCommand> {
    const MIN_KEYWORD_LEN: usize = 5;
    const SIMILARITY_THRESHOLD: f64 = 0.92;
    const MAX_LEN_DIFF_RATIO: f64 = 0.3;

    // Exact match first, always wins regardless of length.
    if let Some(c) = commands
        .iter()
        .find(|c| normalize_text(&c.keyword) == text_normalized)
    {
        return Some(c);
    }

    commands
        .iter()
        .filter_map(|c| {
            let keyword_normalized = normalize_text(&c.keyword);

            if keyword_normalized.len() < MIN_KEYWORD_LEN {
                return None;
            }

            let len_diff =
                (keyword_normalized.len() as isize - text_normalized.len() as isize).abs() as f64;
            let max_len = keyword_normalized.len().max(text_normalized.len()) as f64;
            if max_len == 0.0 || len_diff / max_len > MAX_LEN_DIFF_RATIO {
                return None;
            }

            let score = jaro_winkler(&keyword_normalized, text_normalized);
            if score >= SIMILARITY_THRESHOLD {
                Some((c, score))
            } else {
                None
            }
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .map(|(c, _)| c)
}

#[tauri::command]
pub fn save_custom_command(
    app: tauri::AppHandle,
    state: tauri::State<CustomCommandsState>,
    keyword: String,
) -> Result<(), String> {
    println!("[save_custom_command] called with keyword={:?}", keyword);

    // let keyword = keyword.trim().to_lowercase();
    let keyword = normalize_text(&keyword); // instead of just .trim().to_lowercase()
    if keyword.is_empty() {
        eprintln!("[save_custom_command] rejected: empty keyword");
        return Err("Keyword cannot be empty".into());
    }

    let operations = crate::state::take_last_operations();
    println!(
        "[save_custom_command] retrieved {} last operation(s)",
        operations.len()
    );
    if operations.is_empty() {
        eprintln!("[save_custom_command] rejected: no operations to save");
        return Err("No recent command to save".into());
    }

    let snapshot = {
        let mut commands = state.0.lock().map_err(|e| e.to_string())?;
        if commands.iter().any(|c| c.keyword == keyword) {
            eprintln!("[save_custom_command] rejected: keyword '{keyword}' already exists");
            return Err("A command with this keyword already exists".into());
        }
        commands.push(UserCommand {
            keyword: keyword.clone(),
            operations,
        });
        println!(
            "[save_custom_command] appended '{keyword}', total now {}",
            commands.len()
        );
        commands.clone()
    };

    match save(&app, &snapshot) {
        Ok(()) => {
            println!("[save_custom_command] saved successfully");
            Ok(())
        }
        Err(e) => {
            eprintln!("[save_custom_command] save failed: {e}");
            Err(e)
        }
    }
}
