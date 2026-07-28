use enigo::{
    Direction::{Click, Press, Release},
    Enigo, Key, Keyboard, Settings,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;
use strsim::normalized_levenshtein;

use crate::tts;

#[derive(Deserialize)]
struct LlmResponse {
    intent: String,
    response: String,
    #[serde(default)]
    operations: Vec<Operation>,
}

#[derive(Deserialize)]
struct Operation {
    action: String,
    parameters: OpParams,
}

#[derive(Deserialize, Default)]
struct OpParams {
    #[serde(default)]
    name: String,
    #[serde(default)]
    location: String,
    #[serde(default)]
    parent: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    extension: String,
    #[serde(default)]
    new_name: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    destination: String,
}

static LAST_FOLDER: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

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

fn set_last_folder(path: PathBuf) {
    let m = LAST_FOLDER.get_or_init(|| Mutex::new(None));
    *m.lock().unwrap() = Some(path);
}

fn get_last_folder() -> Option<PathBuf> {
    LAST_FOLDER.get().and_then(|m| m.lock().unwrap().clone())
}

fn get_focused_explorer_path() -> Option<PathBuf> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            r#"
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Win32 {
    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();
}
"@
$hwnd = [Win32]::GetForegroundWindow()
$shell = New-Object -ComObject Shell.Application
foreach ($window in $shell.Windows()) {
    try {
        if ([IntPtr]$window.HWND -eq $hwnd) {
            Write-Output $window.Document.Folder.Self.Path
            break
        }
    } catch {}
}
"#,
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

fn desktop_dir() -> PathBuf {
    dirs::desktop_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// "Smart" default: whatever Explorer folder is currently focused, else Desktop.
fn default_workspace() -> PathBuf {
    let focused = get_focused_explorer_path();
    eprintln!("Focused Explorer path detected: {:?}", focused);
    focused.or_else(get_last_folder).unwrap_or_else(desktop_dir)
}

fn sanitize_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| !matches!(c, '"' | '\'' | '<' | '>' | ':' | '|' | '?' | '*'))
        .collect();

    cleaned
        .trim()
        .trim_end_matches(|c: char| matches!(c, '.' | ',' | '!' | '?'))
        .trim()
        .to_string()
}

/// Strips everything except letters/digits and lowercases — so
/// "test123", "test 1 2 3", and "Test-123!" all normalize identically.
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// Looks inside `dir` for the file/folder that best matches `target`.
/// Tries an exact normalized match first, then falls back to fuzzy
/// similarity. Returns None if nothing is close enough to be confident.
fn find_best_match(target: &str, dir: &Path) -> Option<PathBuf> {
    let target_norm = normalize(target);
    if target_norm.is_empty() {
        return None;
    }

    let entries = fs::read_dir(dir).ok()?;
    let mut best: Option<(PathBuf, f64)> = None;

    for entry in entries.flatten() {
        let path = entry.path();

        // Compare against the name both with and without extension,
        // since spoken commands rarely include the extension.
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let full_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let stem_norm = normalize(&stem);
        let full_norm = normalize(&full_name);

        if stem_norm == target_norm || full_norm == target_norm {
            return Some(path);
        }

        let score = normalized_levenshtein(&target_norm, &stem_norm)
            .max(normalized_levenshtein(&target_norm, &full_norm));

        if best.as_ref().map_or(true, |(_, s)| score > *s) {
            best = Some((path, score));
        }
    }

    best.filter(|(_, score)| *score >= 0.7)
        .map(|(path, _)| path)
}

/// Entry point: parse the LLM's JSON, execute operations in order if any,
/// stop on first failure, and always return a spoken-friendly result.
pub fn handle_llm_response(llm_output: &str) -> String {
    println!("getting handled by rust by llm");
    let cleaned = llm_output
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let parsed: LlmResponse = match serde_json::from_str(cleaned) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to parse LLM response: {e}\nRaw: {cleaned}");
            return "Sorry, I couldn't understand that.".into();
        }
    };

    match parsed.intent.as_str() {
        "chat" | "error" => parsed.response,
        "system" if !parsed.operations.is_empty() => execute_operations(&parsed.operations),
        _ => parsed.response,
    }
}

fn execute_operations(operations: &[Operation]) -> String {
    let mut known_paths: HashMap<String, PathBuf> = HashMap::new();

    for op in operations {
        match execute_operation(op, &mut known_paths) {
            Ok(_) => continue,
            Err(e) => return format!("Command failed: {e}"),
        }
    }

    "Done".into()
}

fn execute_operation(op: &Operation, known: &mut HashMap<String, PathBuf>) -> Result<(), String> {
    let p = &op.parameters;

    match op.action.as_str() {
        "create_folder" => {
            let name = sanitize_name(&p.name);
            if name.is_empty() {
                return Err("no folder name given".into());
            }
            let base = resolve_base(p, known);
            let path = base.join(&name);
            fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            known.insert(name.to_lowercase(), path.clone());
            set_last_folder(path);
            Ok(())
        }
        "create_file" => {
            let mut name = sanitize_name(&p.name);
            if name.is_empty() {
                return Err("no file name given".into());
            }

            if !p.extension.is_empty() {
                let ext = p.extension.trim_start_matches('.');
                // Avoid double extensions if the name already happens to end with it.
                if !name
                    .to_lowercase()
                    .ends_with(&format!(".{}", ext.to_lowercase()))
                {
                    name = format!("{name}.{ext}");
                }
            }

            let base = resolve_base(p, known);
            let path = base.join(&name);
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir).ok();
            }
            fs::write(&path, "").map_err(|e| e.to_string())?;
            known.insert(name.to_lowercase(), path.clone());
            set_last_folder(path.parent().unwrap_or(&base).to_path_buf());
            Ok(())
        }
        "open_folder" | "open_file" => {
            let path = resolve_target(p, known);
            open_with_shell(&path)?;
            if path.is_dir() {
                set_last_folder(path);
            } else if let Some(parent) = path.parent() {
                set_last_folder(parent.to_path_buf());
            }
            Ok(())
        }
        "delete_folder" => {
            let path = resolve_target(p, known);
            fs::remove_dir_all(&path).map_err(|e| e.to_string())
        }
        "delete_file" => {
            let path = resolve_target(p, known);
            fs::remove_file(&path).map_err(|e| e.to_string())
        }
        "rename_folder" | "rename_file" => {
            let path = resolve_target(p, known);
            let new_name = sanitize_name(&p.new_name);
            if new_name.is_empty() {
                return Err("no new name given".into());
            }
            let new_path = path
                .parent()
                .unwrap_or(&default_workspace())
                .join(&new_name);
            fs::rename(&path, &new_path).map_err(|e| e.to_string())?;
            known.insert(new_name.to_lowercase(), new_path);
            Ok(())
        }
        "move_folder" | "move_file" => {
            let src = resolve_named(&p.source, known);
            let dest_base = resolve_named(&p.destination, known);
            let file_name = src.file_name().ok_or("invalid source")?;
            fs::rename(&src, dest_base.join(file_name)).map_err(|e| e.to_string())
        }
        "copy_file" => {
            let src = resolve_named(&p.source, known);
            let dest_base = resolve_named(&p.destination, known);
            let file_name = src.file_name().ok_or("invalid source")?;
            fs::copy(&src, dest_base.join(file_name)).map_err(|e| e.to_string())?;
            Ok(())
        }
        "copy_folder" => {
            // Shallow support only for now — recursive dir copy needs a helper you don't have yet.
            Err("folder copying not supported yet".into())
        }
        "open_app" => {
            let name = sanitize_name(&p.name);
            if name.is_empty() {
                return Err("no app name given".into());
            }
            let status = Command::new("cmd")
                .args(["/C", "start", "", &name])
                .status()
                .map_err(|e| e.to_string())?;
            if status.success() {
                Ok(())
            } else {
                Err(format!("could not open {name}"))
            }
        }
        "close_app" => {
            let name = sanitize_name(&p.name);
            if name.is_empty() {
                return Err("no app name given".into());
            }
            let status = Command::new("taskkill")
                .args(["/IM", &format!("{name}.exe"), "/F"])
                .status()
                .map_err(|e| e.to_string())?;
            if status.success() {
                Ok(())
            } else {
                Err(format!("could not close {name}"))
            }
        }
        other => Err(format!("unknown action: {other}")),
    }
}

fn resolve_base(p: &OpParams, known: &HashMap<String, PathBuf>) -> PathBuf {
    let candidate = if !p.parent.is_empty() && p.parent.to_lowercase() != "desktop" {
        Some(p.parent.as_str())
    } else if !p.location.is_empty() && p.location.to_lowercase() != "desktop" {
        Some(p.location.as_str())
    } else {
        None
    };

    match candidate {
        Some(c) => resolve_named(c, known),
        None => default_workspace(),
    }
}

fn resolve_target(p: &OpParams, known: &HashMap<String, PathBuf>) -> PathBuf {
    if !p.path.is_empty() {
        return PathBuf::from(&p.path);
    }
    let name = if !p.location.is_empty() && p.location.to_lowercase() != "desktop" {
        p.location.clone()
    } else {
        p.name.clone()
    };
    resolve_named(&name, known)
}

fn workspace_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(focused) = get_focused_explorer_path() {
        candidates.push(focused);
    }
    if let Some(last) = get_last_folder() {
        candidates.push(last);
    }
    candidates.push(desktop_dir());

    candidates
}

fn resolve_named(name: &str, known: &HashMap<String, PathBuf>) -> PathBuf {
    let sanitized = sanitize_name(name);

    if let Some(path) = known.get(&sanitized.to_lowercase()) {
        return path.clone();
    }

    for dir in workspace_candidates() {
        if let Some(matched) = find_best_match(&sanitized, &dir) {
            return matched;
        }
    }

    // Genuinely not found anywhere — default to Desktop rather than
    // constructing a nested guess that almost certainly doesn't exist.
    desktop_dir().join(&sanitized)
}

fn open_with_shell(path: &PathBuf) -> Result<(), String> {
    let status = Command::new("cmd")
        .args(["/C", "start", "", path.to_str().unwrap_or("")])
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("could not open {}", path.display()))
    }
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

pub fn execute(text: &str) -> Option<String> {
    let text = clean_transcript(text).to_lowercase();
    let text = text.trim();

    // Compound/long sentences should always fall through to the LLM —
    // the fast-path handlers below assume short, single-intent phrases
    // and will misfire on multi-step commands like this one.
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

    // Give the app a moment to launch and gain focus before typing.
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

    if let Some(name) = raw_target.strip_prefix("folder ") {
        return open_named_item(name);
    }
    if let Some(name) = raw_target.strip_prefix("file ") {
        return open_named_item(name);
    }

    // was: let app_name = text[idx + "open ".len()..].trim();  <-- recomputed raw, no sanitize
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

fn type_text(content: &str) -> String {
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

fn press_combo(enigo: &mut Enigo, modifier: Key, key: Key) {
    let _ = enigo.key(modifier, Press);
    let _ = enigo.key(key, Click);
    let _ = enigo.key(modifier, Release);
}
