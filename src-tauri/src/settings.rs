// settings.rs
// Single source of truth for all persisted app settings.
// One JSON file on disk, one struct in memory, wrapped in a Mutex for Tauri state.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Manager;

use crate::system::types::AppSettings;

fn settings_file_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("settings.json"))
}

/// Load settings from disk. Called once at startup in main.rs.
/// Falls back to AppSettings::default() if the file doesn't exist or is corrupt
/// (corrupt file is logged, not overwritten, so you don't lose data on a bad parse).
pub fn load(app: &tauri::AppHandle) -> AppSettings {
    let path = match settings_file_path(app) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Could not resolve settings path: {e}");
            return AppSettings::default();
        }
    };

    if !path.exists() {
        return AppSettings::default();
    }

    match fs::read_to_string(&path) {
        Ok(raw) => match serde_json::from_str(&raw) {
            Ok(settings) => settings,
            Err(e) => {
                eprintln!("Settings file corrupt, using defaults: {e}");
                AppSettings::default()
            }
        },
        Err(e) => {
            eprintln!("Could not read settings file, using defaults: {e}");
            AppSettings::default()
        }
    }
}

/// Write current settings to disk. Call after any change.
pub fn save(app: &tauri::AppHandle, settings: &AppSettings) -> Result<(), String> {
    let path = settings_file_path(app)?;
    let raw = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(&path, raw).map_err(|e| e.to_string())
}

/// Managed Tauri state — one Mutex wrapping the whole settings struct.
pub struct SettingsState(pub Mutex<AppSettings>);
