use crate::system::app_context;
use crate::system::fast_match::type_text;
use crate::system::helper_functions::{
    default_workspace, is_dangerous, open_with_shell, set_last_folder,
};
use crate::system::path_resolver::{resolve_base, resolve_named, resolve_target, sanitize_name};
use crate::system::types::{LlmResponse, Operation};
use enigo::{
    Direction::{Click, Press, Release},
    Enigo, Key, Keyboard, Settings,
};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

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
        "open_app_in_folder" => {
            let name = sanitize_name(&p.name);
            if name.is_empty() {
                return Err("no app name given".into());
            }
            let folder = resolve_base(p, known);
            app_context::open_app_in_folder(&name, &folder)?;
            Ok(())
        }
        "delete_folder" => {
            let path = resolve_target(p, known);
            if is_dangerous(&path) {
                return Err(format!(
                    "Refusing to delete protected path: {}",
                    path.display()
                ));
            }
            fs::remove_dir_all(&path).map_err(|e| e.to_string())
        }
        "delete_file" => {
            let path = resolve_target(p, known);
            if is_dangerous(&path) {
                return Err(format!(
                    "Refusing to delete protected path: {}",
                    path.display()
                ));
            }
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
        "copy_folder" => Err("folder copying not supported yet".into()),
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
        "press_keys" => press_keys(&p.keys),
        "type" => {
            let result = type_text(&p.text);
            if result.starts_with("Failed") {
                Err(result)
            } else {
                Ok(())
            }
        }
        "wait" => {
            let ms: u64 = p.text.parse().unwrap_or(500);
            std::thread::sleep(std::time::Duration::from_millis(ms));
            Ok(())
        }
        other => Err(format!("unknown action: {other}")),
    }
}

fn parse_key(token: &str) -> Option<Key> {
    match token.to_lowercase().as_str() {
        "ctrl" | "control" => Some(Key::Control),
        "alt" => Some(Key::Alt),
        "shift" => Some(Key::Shift),
        "meta" | "win" | "windows" | "cmd" => Some(Key::Meta),
        "enter" | "return" => Some(Key::Return),
        "tab" => Some(Key::Tab),
        "escape" | "esc" => Some(Key::Escape),
        "space" => Some(Key::Space),
        "backspace" => Some(Key::Backspace),
        "delete" => Some(Key::Delete),
        s if s.chars().count() == 1 => Some(Key::Unicode(s.chars().next().unwrap())),
        _ => None,
    }
}

fn press_keys(combo: &str) -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    let tokens: Vec<&str> = combo
        .split('+')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if tokens.is_empty() {
        return Err("no keys given".into());
    }

    let keys: Vec<Key> = tokens
        .iter()
        .map(|t| parse_key(t).ok_or_else(|| format!("unknown key: {t}")))
        .collect::<Result<_, _>>()?;
    let (modifiers, main) = keys.split_at(keys.len() - 1);
    let main_key = main[0];

    for m in modifiers {
        enigo.key(*m, Press).map_err(|e| e.to_string())?;
    }
    enigo.key(main_key, Click).map_err(|e| e.to_string())?;
    for m in modifiers.iter().rev() {
        enigo.key(*m, Release).map_err(|e| e.to_string())?;
    }
    Ok(())
}
