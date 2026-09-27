use crate::system::types::Operation;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter};

/// User's selected backend. `None` = LLM disabled entirely (deterministic matcher only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmChoice {
    Groq,
    None,
}

impl Default for LlmChoice {
    fn default() -> Self {
        LlmChoice::Groq
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Idle,
    Recording,
    Transcribing,
    Thinking,      // LLM call in progress
    ScanningImage, // if/when vision step is added
    Executing,     // running the actual system action
    Speaking,      // TTS playback
    Error,
}

static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

pub fn init(app: AppHandle) {
    let _ = APP_HANDLE.set(app);
}

pub fn set_status(status: TaskStatus) {
    if let Some(app) = APP_HANDLE.get() {
        let _ = app.emit("task-status", status);
    }
}

static LAST_OPERATIONS: Mutex<Vec<Operation>> = Mutex::new(Vec::new());

pub fn set_last_operations(ops: Vec<Operation>) {
    println!(
        "[state::set_last_operations] storing {} operation(s)",
        ops.len()
    );
    if let Ok(mut guard) = LAST_OPERATIONS.lock() {
        *guard = ops;
    } else {
        eprintln!("[state::set_last_operations] failed to lock LAST_OPERATIONS");
    }
}

pub fn take_last_operations() -> Vec<Operation> {
    match LAST_OPERATIONS.lock() {
        Ok(g) => {
            println!(
                "[state::take_last_operations] returning {} operation(s)",
                g.len()
            );
            g.clone()
        }
        Err(e) => {
            eprintln!("[state::take_last_operations] lock failed: {e}");
            Vec::new()
        }
    }
}
