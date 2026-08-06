use crate::system::helper_functions::{
    get_selected_explorer_item, open_with_shell, set_last_folder, workspace_candidates,
};
use crate::system::path_resolver::{find_best_match, sanitize_name};
use crate::tts;
use enigo::{
    Direction::{Click, Press, Release},
    Enigo, Key, Keyboard, Settings,
};
use std::process::Command;
use std::thread;
use std::time::Duration;

fn clean_transcript(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

pub fn execute(text: &str) -> Option<String> {
    let text = clean_transcript(text).to_lowercase();
    let text = text.trim();

    if word_count(text) > 3 {
        return None;
    }

    println!("getting executed by rust handler");

    if let Some(r) = handle_open_and_type(text) {
        return Some(r);
    }
    if let Some(rest) = text.strip_prefix("type ") {
        return Some(type_text(rest));
    }
    if let Some(rest) = text.strip_prefix("search ") {
        return Some(match search_windows(rest) {
            Ok(_) => format!("Searching for {}", rest),
            Err(e) => format!("Search failed: {}", e),
        });
    }
    if let Some(r) = handle_open_app(text) {
        return Some(r);
    }
    if let Some(r) = handle_browser_action(text) {
        return Some(r);
    }
    if let Some(r) = handle_key_press(text) {
        return Some(r);
    }

    println!("returning none by rust handler");
    None
}

fn handle_open_and_type(text: &str) -> Option<String> {
    let (open_part, type_part) = text.split_once(" and type ")?;
    let open_result = handle_open_app(open_part.trim())?;
    thread::sleep(Duration::from_millis(800));
    let typed = type_text(type_part.trim());
    Some(format!("{open_result}, then {typed}"))
}

fn handle_open_app(text: &str) -> Option<String> {
    let idx = text.find("open ")?;
    let raw_target = text[idx + "open ".len()..].trim();
    if raw_target.is_empty() {
        return None;
    }

    if matches!(
        raw_target,
        "this file" | "selected file" | "file" | "this" | "selected item" | "it"
    ) {
        if let Some(path) = get_selected_explorer_item() {
            return match open_with_shell(&path) {
                Ok(_) => Some(format!("Opening {}", path.display())),
                Err(e) => Some(format!("Command failed: {e}")),
            };
        }
        return Some("No file selected".into());
    }

    if let Some(name) = raw_target.strip_prefix("folder ") {
        return open_named_item(name);
    }
    if let Some(name) = raw_target.strip_prefix("file ") {
        return open_named_item(name);
    }

    let app_name = sanitize_name(raw_target);
    if app_name.is_empty() {
        return None;
    }

    let status = Command::new("cmd")
        .args(["/C", "start", "", &app_name])
        .status();
    match status {
        Ok(s) if s.success() => Some(format!("Opening {app_name}")),
        _ => Some(format!("Command failed: could not open {app_name}")),
    }
}

fn open_named_item(name: &str) -> Option<String> {
    let sanitized = sanitize_name(name);
    if sanitized.is_empty() {
        return None;
    }

    for dir in workspace_candidates() {
        if let Some(matched) = find_best_match(&sanitized, &dir) {
            return match open_with_shell(&matched) {
                Ok(_) => {
                    if matched.is_dir() {
                        set_last_folder(matched);
                    }
                    Some(format!("Opening {sanitized}"))
                }
                Err(e) => Some(format!("Command failed: {e}")),
            };
        }
    }

    Some(format!("Could not find {sanitized}"))
}

fn handle_browser_action(text: &str) -> Option<String> {
    let mut enigo = Enigo::new(&Settings::default()).ok()?;

    if text.contains("new tab") {
        press_combo(&mut enigo, Key::Control, Key::Unicode('t'));
        return Some("Opened new tab".into());
    }
    if text.contains("close tab") {
        press_combo(&mut enigo, Key::Control, Key::Unicode('w'));
        return Some("Closed tab".into());
    }
    if text.contains("next tab") || text.contains("switch tab") {
        press_combo(&mut enigo, Key::Control, Key::Tab);
        return Some("Switched tab".into());
    }
    if text.contains("new window") {
        press_combo(&mut enigo, Key::Control, Key::Unicode('n'));
        return Some("Opened new window".into());
    }
    None
}

fn handle_key_press(text: &str) -> Option<String> {
    let mut enigo = Enigo::new(&Settings::default()).ok()?;
    let words: Vec<&str> = text.split_whitespace().collect();
    let has = |w: &str| words.contains(&w);

    if text.contains("select all") {
        press_combo(&mut enigo, Key::Control, Key::Unicode('a'));
        let _ = tts::speak("Selecetd ");
        return Some("Selected all".into());
    }

    let (label, key) = if has("enter") {
        ("Enter", Key::Return)
    } else if has("tab") {
        ("Tab", Key::Tab)
    } else if has("escape") {
        ("Escape", Key::Escape)
    } else if has("space") {
        ("Space", Key::Space)
    } else if has("backspace") {
        ("Backspace", Key::Backspace)
    } else if has("delete") {
        ("Delete", Key::Delete)
    } else if has("copy") {
        press_combo(&mut enigo, Key::Control, Key::Unicode('c'));
        let _ = tts::speak("Copied ");
        return Some("Copied".into());
    } else if has("paste") {
        press_combo(&mut enigo, Key::Control, Key::Unicode('v'));
        let _ = tts::speak("Pasted ");
        return Some("Pasted".into());
    } else if has("save") {
        press_combo(&mut enigo, Key::Control, Key::Unicode('s'));
        let _ = tts::speak("Saved ");
        return Some("Saved".into());
    } else {
        return None;
    };

    enigo.key(key, Click).ok()?;
    Some(format!("Pressed {label}"))
}

fn press_combo(enigo: &mut Enigo, modifier: Key, key: Key) {
    let _ = enigo.key(modifier, Press);
    let _ = enigo.key(key, Click);
    let _ = enigo.key(modifier, Release);
}

pub fn type_text(content: &str) -> String {
    if content.is_empty() {
        return "Nothing to type".into();
    }
    match Enigo::new(&Settings::default()) {
        Ok(mut enigo) => match enigo.text(content) {
            Ok(_) => format!("Typed: {content}"),
            Err(e) => format!("Failed to type: {e}"),
        },
        Err(e) => format!("Failed to init input: {e}"),
    }
}

fn search_windows(query: &str) -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    enigo.key(Key::Meta, Press).map_err(|e| e.to_string())?;
    enigo.key(Key::Meta, Release).map_err(|e| e.to_string())?;
    thread::sleep(Duration::from_millis(300));
    enigo.text(query).map_err(|e| e.to_string())?;
    Ok(())
}
