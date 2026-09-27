mod audio;
mod commands;
mod custom_command;
mod find_serial;
mod llm_provider;
mod resources;
mod settings;
mod state;
mod system;
mod tts;
mod ui_callbacks;
mod whisper;
mod whisper_engine;

use crate::ui_callbacks::check_internet;
use audio::AudioState;
use custom_command::{save_custom_command, CustomCommandsState};
use settings::SettingsState;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use whisper_engine::{WhisperEngine, WhisperState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AudioState::new())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            // App data dir setup
            let app_data_dir = app.path().app_data_dir().expect("no app data dir");
            std::fs::create_dir_all(&app_data_dir).ok();
            system::helper_functions::init_discovered_apps();

            // Memory state (rolling chat history + user_data.md/memory.md)
            let memory_state = system::memory::load_memory_state(&app_data_dir);
            app.manage(system::types::MemoryStateHandle(Mutex::new(memory_state)));

            // Load .env for API keys
            if let Ok(env_path) = app
                .path()
                .resolve(".env", tauri::path::BaseDirectory::Resource)
            {
                let _ = dotenvy::from_path(&env_path);
            }

            // System prompt / resources init
            resources::init(app.handle()).expect("Failed to load system prompt");

            // Settings + custom commands state
            app.manage(SettingsState(Mutex::new(settings::load(app.handle()))));
            app.manage(CustomCommandsState(Mutex::new(custom_command::load(
                app.handle(),
            ))));

            // Whisper STT engine init
            let model_path = app
                .path()
                .resolve(
                    "resources/models/ggml-base.bin",
                    tauri::path::BaseDirectory::Resource,
                )
                .expect("Failed to resolve whisper model path");
            let engine = WhisperEngine::new(model_path.to_str().expect("invalid model path"))
                .expect("Failed to load whisper model");
            app.manage(WhisperState(Arc::new(Mutex::new(engine))));

            // App-wide handle + serial (AirGrip) listener
            let handle = app.handle().clone();
            state::init(handle.clone());
            audio::start_serial_listener(handle.clone());

            let discovery_dir = app_data_dir.clone();
            tauri::async_runtime::spawn(async move {
                system::app_discovery::sync_app_aliases(&discovery_dir).await;
            });

            // Global shortcut: Ctrl+Alt+Shift+9 toggles recording
            let toggle_shortcut = Shortcut::new(
                Some(Modifiers::ALT | Modifiers::CONTROL | Modifiers::SHIFT),
                Code::Digit9,
            );

            let handle = app.handle().clone();
            app.global_shortcut()
                .on_shortcut(toggle_shortcut, move |_app, _shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }

                    let handle = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        let state = handle.state::<AudioState>();
                        let is_recording = audio::is_recording(&state);

                        if !is_recording {
                            let _ = audio::start_recording(&state);
                        } else {
                            if audio::stop_recording(&state).is_err() {
                                return;
                            }

                            let samples = audio::take_samples(&state);
                            let llm_choice = {
                                let settings_state = handle.state::<SettingsState>();
                                let settings = settings_state.0.lock().unwrap();
                                settings.llm_choice
                            };

                            if let Ok(result) =
                                commands::run_transcribe_only(&handle, llm_choice, samples).await
                            {
                                let _ = handle.emit("command-result", result);
                            }
                        }
                    });
                })?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            check_internet,
            save_custom_command,
        ])
        .run(tauri::generate_context!())
        .expect("error running app");
}
