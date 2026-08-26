use crate::commands::normalize_text;
use crate::system::types::Operation;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
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
            Ok(commands) => {

                commands
            }
            Err(e) => {
                Vec::new()
            }
        },
        Err(e) => {
            Vec::new()
        }
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
