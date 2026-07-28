use serde_json::Value;
use std::{fs, sync::OnceLock};

#[derive(Debug)]
pub struct AppResources {
    pub system_prompt: String,
    pub response_schema: Value,
}

static RESOURCES: OnceLock<AppResources> = OnceLock::new();

pub fn init() -> Result<(), Box<dyn std::error::Error>> {
    let system_prompt =
        fs::read_to_string("resources/prompts/system.txt")?;

    let response_schema: Value = serde_json::from_str(
        &fs::read_to_string("resources/schemas/response_schema.json")?,
    )?;

    RESOURCES
        .set(AppResources {
            system_prompt,
            response_schema,
        })
        .expect("Resources already initialized");

    Ok(())
}

pub fn get() -> &'static AppResources {
    RESOURCES
        .get()
        .expect("Resources not initialized")
}