use crate::whisper_engine::WhisperState;
use anyhow::Result;
use tauri::Manager;

pub async fn transcribe(
    app: &tauri::AppHandle,
    samples: Vec<f32>,
    language: &str,
) -> Result<String> {
    let language = language.to_string();
    let state = app.state::<WhisperState>();
    let engine_arc = state.0.clone();

    let text = tauri::async_runtime::spawn_blocking(move || {
        let engine = engine_arc.lock().unwrap();
        engine.transcribe(&samples, &language)
    })
    .await??;

    println!("text : {}", text);
    Ok(text)
}