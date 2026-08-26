use crate::llm_provider;
use crate::state::{self, TaskStatus};
use crate::system::app_context;
use crate::system::browser_automation;
use crate::system::fast_match;
use crate::system::helper_functions::{
    default_workspace, is_dangerous, open_with_shell, resolve_operation_app_name,
};
use crate::system::path_resolver::{
    resolve_base, resolve_full_path, resolve_target, sanitize_name,
};
use crate::system::types::{LlmResponse, Operation};
use crate::tts;
use std::fs;
use std::process::Command;
use tauri::AppHandle;

use super::helper_functions;

pub fn handle_llm_response(app: &AppHandle, llm_output: &str) -> String {
    let cleaned = llm_output
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    println!("{}", cleaned);

    let mut parsed: LlmResponse = match serde_json::from_str(cleaned) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            let msg = "Sorry, I couldn't understand that.".to_string();
            let _ = tts::speak(app, &msg);
            return msg;
        }
    };

    // Normalize: pull any stray nested "parameters" values into flat fields
    // Normalize: pull any stray nested "parameters" values into flat fields
    parsed.operations = parsed
        .operations
        .into_iter()
        .map(Operation::normalize)
        .map(resolve_operation_app_name)
        .collect();

    let result = match parsed.intent.as_str() {
        "chat" | "error" => parsed.response,
        "system" if !parsed.operations.is_empty() => {
            println!(
                "[handle_llm_response] executing {} operation(s)",
                parsed.operations.len()
            );
            state::set_status(TaskStatus::Executing);
            let result = execute_operations(app, &parsed.operations);
            state::set_last_operations(parsed.operations.clone());
            println!("[handle_llm_response] execution result: {result}");
            result
        }
        _ => parsed.response,
    };

    state::set_status(TaskStatus::Speaking);
    if let Err(e) = tts::speak(app, &result) {
        eprintln!("TTS speak failed: {e}");
    }

    result
}

pub fn execute_operations(app: &AppHandle, operations: &[Operation]) -> String {
    for op in operations {
        match execute_operation(app, op) {
            Ok(_) => continue,
            Err(e) => return format!("Command failed: {e}"),
        }
    }
    "Done".into()
}

fn execute_operation(app: &AppHandle, p: &Operation) -> Result<(), String> {
    match p.action.as_str() {
        "create_folder" => {
            let name = sanitize_name(&p.name);
            if name.is_empty() {
                return Err("no folder name given".into());
            }
            let base = resolve_base(&p.location);
            let path = base.join(&name);
            fs::create_dir_all(&path).map_err(|e| e.to_string())
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
            let base = resolve_base(&p.location);
            let path = base.join(&name);
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir).ok();
            }
            fs::write(&path, &p.text).map_err(|e| e.to_string())
        }
        "write_file" => {
            let path = resolve_target(&p.location, &p.name, &p.extension);

            if !path.exists() {
                return Err(format!("file not found: {}", path.display()));
            }

            fs::write(&path, &p.text).map_err(|e| e.to_string())
        }
        "open_folder" | "open_file" => {
            let path = resolve_target(&p.location, &p.name, &p.extension);
            open_with_shell(&path)
        }
        "delete_folder" => {
            let path = resolve_target(&p.location, &p.name, &p.extension);
            if is_dangerous(&path) {
                return Err(format!(
                    "Refusing to delete protected path: {}",
                    path.display()
                ));
            }
            fs::remove_dir_all(&path).map_err(|e| e.to_string())
        }
        "delete_file" => {
            let path = resolve_target(&p.location, &p.name, &p.extension);
            if is_dangerous(&path) {
                return Err(format!(
                    "Refusing to delete protected path: {}",
                    path.display()
                ));
            }
            fs::remove_file(&path).map_err(|e| e.to_string())
        }
        "rename_folder" | "rename_file" => {
            let path = resolve_target(&p.location, &p.name, &p.extension);
            let new_name = sanitize_name(&p.new_name);
            if new_name.is_empty() {
                return Err("no new name given".into());
            }
            let new_path = path
                .parent()
                .unwrap_or(&default_workspace())
                .join(&new_name);
            fs::rename(&path, &new_path).map_err(|e| e.to_string())
        }
        "move_folder" | "move_file" => {
            let src = resolve_full_path(&p.source);
            let dest_base = resolve_full_path(&p.destination);
            let file_name = src.file_name().ok_or("invalid source")?;
            fs::rename(&src, dest_base.join(file_name)).map_err(|e| e.to_string())
        }
        "copy_file" => {
            let src = resolve_full_path(&p.source);
            let dest_base = resolve_full_path(&p.destination);
            let file_name = src.file_name().ok_or("invalid source")?;
            fs::copy(&src, dest_base.join(file_name)).map_err(|e| e.to_string())?;
            Ok(())
        }
        "copy_folder" => Err("folder copying not supported yet".into()),
        "open_app_in_folder" => {
            let name = sanitize_name(&p.name);
            if name.is_empty() {
                return Err("no app name given".into());
            }
            let folder = if p.location.is_empty() {
                crate::system::helper_functions::get_focused_explorer_path()
                    .unwrap_or_else(crate::system::helper_functions::desktop_dir)
            } else {
                resolve_base(&p.location)
            };
            app_context::open_app_in_folder(&name, &folder)?;
            Ok(())
        }
        "open_app" => {
            let name = sanitize_name(&p.name);
            if name.is_empty() {
                return Err("no app name given".into());
            }

            if !p.location.is_empty() {
                // location given -> behave like open_app_in_folder
                let folder = resolve_base(&p.location);
                app_context::open_app_in_folder(&name, &folder)?;
                Ok(())
            } else {
                // no location -> simple open
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
        "browser_search" => {
            let browser = if p.name.is_empty() {
                None
            } else {
                Some(p.name.as_str())
            };
            helper_functions::open_browser_search(browser, &p.text).map_err(|e| e.to_string())?;
            Ok(())
        }
        "browser_open_first_result" => {
            let browser_name = if p.name.is_empty() {
                "chrome"
            } else {
                p.name.as_str()
            };
            let exe_path = helper_functions::resolve_browser_exe_path(browser_name)
                .ok_or_else(|| format!("could not locate {browser_name}"))?;
            let query = p.text.clone();

            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async move {
                    browser_automation::search_and_open_first(&exe_path, &query, true).await
                })
            })
            .map_err(|e| e.to_string())?;
            Ok(())
        }
        "vision_query" => {
            let img_b64 = helper_functions::capture_screen_base64()?;
            let question = p.text.clone();

            let answer = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async move {
                    llm_provider::query_vision_llm(&reqwest::Client::new(), &img_b64, &question)
                        .await
                })
            })
            .map_err(|e| e.to_string())?;
            let _ = tts::speak(app, &answer);

            Ok(())
        }
        "contextual_search" => {
            helper_functions::contextual_search(&p.location, &p.text).map_err(|e| e.to_string())?;
            Ok(())
        }
        "type_text" => fast_match::type_text(&p.text),
        "volume_up" => fast_match::volume_up(),
        "volume_down" => fast_match::volume_down(),
        "mute" => fast_match::mute(),
        "lock_screen" => fast_match::lock_screen(),
        "sleep" => fast_match::sleep(),
        "shutdown" => fast_match::shutdown(),
        "restart" => fast_match::restart(),
        "screenshot" => fast_match::screenshot(),
        "copy" => fast_match::copy(),
        "paste" => fast_match::paste(),
        "select_all" => fast_match::select_all(),
        "save" => fast_match::save(),
        "new_tab" => fast_match::new_tab(),
        "close_tab" => fast_match::close_tab(),
        "new_window" => fast_match::new_window(),
        "press_keys" => fast_match::press_keys(&p.keys),
        "undo" => fast_match::undo(),
        "redo" => fast_match::redo(),
        "media_play_pause" => fast_match::media_play_pause(),
        "media_next" => fast_match::media_next(),
        "media_prev" => fast_match::media_prev(),
        "scroll_up" => fast_match::scroll_up(),
        "scroll_down" => fast_match::scroll_down(),
        "switch_tab" => fast_match::switch_tab(),
        "go_back" => fast_match::go_back(),
        "go_forward" => fast_match::go_forward(),
        "maximize_window" => fast_match::maximize_focused_window(),
        "minimize_window" => {
            if p.name.is_empty() {
                fast_match::minimize_focused_window()
            } else {
                fast_match::minimize_window_by_app(&p.name)
            }
        }
        "minimize_all" => fast_match::minimize_all_windows(),
        other => Err(format!("unknown action: {other}")),
    }
}
