mod audio;
mod commands;
mod custom_command;
mod find_serial;
mod llm_provider;
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
use state::{
    get_current_language, set_current_language, set_selected_languages, toggle_language, TaskStatus,
};
use std::sync::Arc;
use std::sync::Mutex;
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use whisper_engine::{WhisperEngine, WhisperState};
mod resources;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // resources::init() moved into .setup() — it needs an AppHandle to resolve paths correctly

    tauri::Builder::default()
        .manage(AudioState::new())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            if let Ok(env_path) = app
                .path()
                .resolve(".env", tauri::path::BaseDirectory::Resource)
            {
                match dotenvy::from_path(&env_path) {
                    Ok(()) => println!("[env] loaded .env from {:?}", env_path),
                    Err(e) => eprintln!("[env] failed to load .env from {:?}: {e}", env_path),
                }
            } else {
                eprintln!("[env] could not resolve .env resource path");
            }

            resources::init(app.handle()).expect("Failed to load system prompt");

            let loaded_settings = settings::load(app.handle());
            app.manage(SettingsState(Mutex::new(loaded_settings)));

            app.manage(CustomCommandsState(Mutex::new(custom_command::load(
                app.handle(),
            ))));

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

            let handle = app.handle().clone();
            state::init(handle.clone());
            audio::start_serial_listener(handle.clone());

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
                            eprintln!("[shortcut] starting recording");
                            if let Err(e) = audio::start_recording(&state) {
                                eprintln!("[shortcut] start failed: {e}");
                            }
                        } else {
                            eprintln!("[shortcut] stopping recording");
                            if let Err(e) = audio::stop_recording(&state) {
                                eprintln!("[shortcut] stop failed: {e}");
                                return;
                            }

                            let samples = audio::take_samples(&state);

                            let (llm_choice, keys, language) = {
                                let settings_state = handle.state::<SettingsState>();
                                let settings = settings_state.0.lock().unwrap();
                                (
                                    settings.llm_choice,
                                    settings.api_keys.clone(),
                                    settings.current_language().to_string(),
                                )
                            };

                            match commands::run_transcribe_only(
                                &handle, llm_choice, &keys, &language, samples,
                            )
                            .await
                            {
                                Ok(result) => {
                                    eprintln!("[shortcut] result: {result}");
                                    let _ = handle.emit("command-result", result);
                                }
                                Err(e) => eprintln!("[shortcut] failed: {e}"),
                            }
                        }
                    });
                })?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_current_language,
            toggle_language,
            set_selected_languages,
            set_current_language,
            check_internet,
            save_custom_command,
        ])
        .run(tauri::generate_context!())
        .expect("error running app");
}
