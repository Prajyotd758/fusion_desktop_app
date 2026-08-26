use serde_json::Value;
use std::{fs, sync::OnceLock};
use tauri::{AppHandle, Manager};

#[derive(Debug)]
pub struct AppResources {
    pub system_prompt: String,
    pub response_schema: Value,
}

static RESOURCES: OnceLock<AppResources> = OnceLock::new();

pub fn init(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let prompt_path = app.path().resolve(
        "resources/prompts/system.txt",
        tauri::path::BaseDirectory::Resource,
    )?;
    let system_prompt = std::fs::read_to_string(&prompt_path)?;

    let response_schema: Value = serde_json::from_str(&fs::read_to_string(
        "resources/schemas/response_schema.json",
    )?)?;

    RESOURCES
        .set(AppResources {
            system_prompt,
            response_schema,
        })
        .expect("Resources already initialized");

    Ok(())
}

pub fn get() -> &'static AppResources {
    RESOURCES.get().expect("Resources not initialized")
}
