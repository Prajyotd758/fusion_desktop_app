use super::types::{ChatTurn, MemoryState, RememberItem};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_HISTORY_TURNS: usize = 6;
const HISTORY_EXPIRY_SECS: i64 = 24 * 60 * 60; // 24h

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn parse_md_list(path: &PathBuf) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.strip_prefix("- ").map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect()
}

fn append_md_line(path: &PathBuf, content: &str) -> std::io::Result<()> {
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "- {}", content)
}

pub fn load_memory_state(app_data_dir: &PathBuf) -> MemoryState {
    let user_data_path = app_data_dir.join("user_data.md");
    let memory_path = app_data_dir.join("memory.md");

    MemoryState {
        user_data: parse_md_list(&user_data_path),
        memory: parse_md_list(&memory_path),
        chat_history: std::collections::VecDeque::new(),
    }
}

/// Call after each successful online dispatch that returns `remember` items.
pub fn apply_remember(
    app_data_dir: &PathBuf,
    state: &Mutex<MemoryState>,
    items: Vec<RememberItem>,
) {
    if items.is_empty() {
        return;
    }
    let user_data_path = app_data_dir.join("user_data.md");
    let memory_path = app_data_dir.join("memory.md");

    let mut guard = state.lock().unwrap();
    for item in items {
        let path = match item.target.as_str() {
            "user_data" => &user_data_path,
            _ => &memory_path,
        };
        if append_md_line(path, &item.content).is_ok() {
            match item.target.as_str() {
                "user_data" => guard.user_data.push(item.content),
                _ => guard.memory.push(item.content),
            }
        }
    }
}

/// Call after every turn (user + assistant), online or offline.
pub fn push_chat_turn(state: &Mutex<MemoryState>, role: &str, content: &str) {
    let mut guard = state.lock().unwrap();
    let cutoff = now() - HISTORY_EXPIRY_SECS;
    guard.chat_history.retain(|t| t.timestamp > cutoff);

    guard.chat_history.push_back(ChatTurn {
        role: role.to_string(),
        content: content.to_string(),
        timestamp: now(),
    });

    while guard.chat_history.len() > MAX_HISTORY_TURNS {
        guard.chat_history.pop_front();
    }

    println!(
        "[push_chat_turn] added {}: \"{}\" (history now has {} turn(s))",
        role,
        content,
        guard.chat_history.len()
    );
}

/// Build the context block injected into the LLM system/user prompt.
pub fn build_memory_context(state: &Mutex<MemoryState>) -> String {
    let guard = state.lock().unwrap();
    let mut out = String::new();

    if !guard.user_data.is_empty() {
        out.push_str("User data:\n");
        for line in &guard.user_data {
            out.push_str(&format!("- {}\n", line));
        }
    }
    if !guard.memory.is_empty() {
        out.push_str("Remembered:\n");
        for line in &guard.memory {
            out.push_str(&format!("- {}\n", line));
        }
    }
    if !guard.chat_history.is_empty() {
        out.push_str("Recent conversation:\n");
        for turn in &guard.chat_history {
            out.push_str(&format!("{}: {}\n", turn.role, turn.content));
        }
    }

    println!("========== MEMORY CONTEXT ==========");
    println!("{}", out);
    println!("=====================================");

    out
}
