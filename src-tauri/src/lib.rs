mod audio;
mod commands;
mod llm;
mod system;
mod tts;
mod whisper;
mod whisper_server;
use audio::AudioState;
use std::sync::Arc;
use tauri::Manager;
mod resources;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    resources::init().expect("Failed to load system prompt");
    whisper_server::start_llama_server();
    whisper_server::start_whisper_server();

    tauri::Builder::default()
        .manage(Arc::new(AudioState::new()))
        .setup(|app| {
            let handle = app.handle().clone();
            audio::start_serial_listener(handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![])
        .run(tauri::generate_context!())
        .expect("error running app");
}