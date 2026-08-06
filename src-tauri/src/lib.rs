mod audio;
mod commands;
mod find_serial;
mod llm;
mod llm_provider;
mod settings;
mod state;
mod system;
mod tts;
mod whisper;
mod whisper_server;
use audio::AudioState;
use settings::SettingsState;
use state::{
    get_current_language, get_llm_choice, set_current_language, set_llm_choice,
    set_selected_languages, toggle_language,
};
use std::sync::Mutex;
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
mod resources;
use crate::system::helper_functions::get_focused_explorer_path;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    resources::init().expect("Failed to load system prompt");
    whisper_server::start_llama_server();
    whisper_server::start_whisper_server();

    tauri::Builder::default()
        .manage(AudioState::new())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            // Load persisted settings (llm_choice, api_keys, etc.) once at startup.
            let loaded_settings = settings::load(app.handle());
            app.manage(SettingsState(Mutex::new(loaded_settings)));

            let handle = app.handle().clone();
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

                            // Pull the current llm_choice + api_keys from persisted settings
                            // right before running the transcription pipeline.
                            let (llm_choice, keys, language) = {
                                let settings_state = handle.state::<SettingsState>();
                                let settings = settings_state.0.lock().unwrap();
                                (
                                    settings.llm_choice,
                                    settings.api_keys.clone(),
                                    settings.current_language().to_string(),
                                )
                            };
                            
                            match commands::run_transcribe_only(llm_choice, &keys, &language).await
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
            get_focused_explorer_path,
            set_llm_choice,
            get_llm_choice,
        ])
        .run(tauri::generate_context!())
        .expect("error running app");
}
