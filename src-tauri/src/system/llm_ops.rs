use crate::browser;
use crate::llm_provider;
use crate::scheduler;
use crate::state::{self, TaskStatus};
use crate::system::app_context;
use crate::system::fast_match;
use crate::system::helper_functions::{
    default_workspace, is_dangerous, open_resolved, open_with_shell, resolve_app,
};
use crate::system::path_resolver::{
    resolve_base, resolve_full_path, resolve_target, sanitize_name,
};
use crate::system::types::MemoryStateHandle;
use crate::system::types::{LlmResponse, Operation};
use crate::tts;
use std::fs;
use std::process::Command;
use tauri::AppHandle;
use tauri::Manager;

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
            let _ = tts::speak(app, &msg, "en");
            return msg;
        }
    };

    // Normalize: pull any stray nested "parameters" values into flat fields
    parsed.operations = parsed
        .operations
        .into_iter()
        .map(Operation::normalize)
        .collect();

    let lang = parsed.response_language.clone();

    // --- memory: apply remember items (before execution, so it survives regardless of op outcome) ---
    if !parsed.remember.is_empty() {
        let app_data_dir = app.path().app_data_dir().expect("no app data dir");
        let memory_state = app.state::<MemoryStateHandle>();
        crate::system::memory::apply_remember(
            &app_data_dir,
            &memory_state.0,
            parsed.remember.clone(),
        );
    }

    let result = match parsed.intent.as_str() {
        "chat" | "error" => parsed.response,
        "system" if !parsed.operations.is_empty() => {
            println!(
                "[handle_llm_response] executing {} operation(s)",
                parsed.operations.len()
            );
            state::set_status(TaskStatus::Executing);
            let result = execute_operations(app, &parsed.operations, &lang, &parsed.response);
            state::set_last_operations(parsed.operations.clone());
            result
        }
        _ => parsed.response,
    };

    // --- memory: push this turn into rolling chat history ---
    {
        let memory_state = app.state::<MemoryStateHandle>();
        crate::system::memory::push_chat_turn(&memory_state.0, "assistant", &result);
    }

    state::set_status(TaskStatus::Speaking);
    if let Err(e) = tts::speak(app, &result, &lang) {
        eprintln!("TTS speak failed: {e}");
    }
    result
}

pub fn execute_operations(
    app: &AppHandle,
    operations: &[Operation],
    lang: &str,
    fallback: &str,
) -> String {
    for op in operations {
        match execute_operation(app, op, lang) {
            Ok(_) => continue,
            Err(e) => return format!("Command failed: {e}"),
        }
    }
    fallback.to_string()
}

pub fn execute_operation(app: &AppHandle, p: &Operation, lang: &str) -> Result<(), String> {
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
        "open_folder" | "open_file" => {
            let path = resolve_target(&p.location, &p.name, &p.extension);
            open_with_shell(&path)
        }
        "open_app_in_folder" => {
            let name = sanitize_name(&p.name);
            eprintln!(
                "[open_app_in_folder] llm name={:?} sanitized={name:?} location={:?}",
                p.name, p.location
            );
            if name.is_empty() {
                eprintln!("[open_app_in_folder] FAIL: empty app name");
                return Err("no app name given".into());
            }
            let folder = if p.location.is_empty() {
                let f = crate::system::helper_functions::get_focused_explorer_path();
                eprintln!("[open_app_in_folder] no location given, focused explorer path = {f:?}");
                f.unwrap_or_else(crate::system::helper_functions::desktop_dir)
            } else {
                resolve_base(&p.location)
            };
            eprintln!(
                "[open_app_in_folder] opening {name:?} in {}",
                folder.display()
            );
            match app_context::open_app_in_folder(&name, &folder) {
                Ok(m) => eprintln!("[open_app_in_folder] OK: {m}"),
                Err(e) => {
                    eprintln!("[open_app_in_folder] FAIL: {e}");
                    return Err(e);
                }
            }
            Ok(())
        }
        "open_app" => {
            let raw = p.name.trim().to_lowercase();
            let name = sanitize_name(&p.name);
            eprintln!(
                "[open_app] llm name={:?} raw={raw:?} sanitized={name:?} location={:?}",
                p.name, p.location
            );
            if name.is_empty() {
                eprintln!("[open_app] FAIL: empty app name");
                return Err("no app name given".into());
            }

            if !p.location.is_empty() {
                let folder = resolve_base(&p.location);
                eprintln!("[open_app] opening {name:?} in folder {}", folder.display());
                let r = app_context::open_app_in_folder(&name, &folder);
                match &r {
                    Ok(m) => eprintln!("[open_app] OK: {m}"),
                    Err(e) => eprintln!("[open_app] FAIL (in folder): {e}"),
                }
                r?;
                Ok(())
            } else {
                let (kind, target) = match resolve_app(&raw).or_else(|| resolve_app(&name)) {
                    Some(r) => r,
                    None => {
                        eprintln!("[open_app] FAIL: could not resolve {raw:?} / {name:?}");
                        return Err(format!("could not resolve app: {name}"));
                    }
                };
                eprintln!("[open_app] launching kind={kind:?} target={target:?}");
                match open_resolved(&kind, &target) {
                    Ok(_) => eprintln!("[open_app] OK: launched {name:?}"),
                    Err(e) => {
                        eprintln!("[open_app] FAIL: launch error for {target:?}: {e}");
                        return Err(e.to_string());
                    }
                }
                Ok(())
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
            let name = if p.name.is_empty() {
                "chrome"
            } else {
                p.name.as_str()
            };
            let exe = browser::resolve_browser_exe_path(name)
                .ok_or_else(|| format!("could not locate {name}"))?;
            let q = p.text.clone();
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(browser::search(&exe, &q))
            })
            .map_err(|e| e.to_string())?;
            Ok(())
        }
        "browser_open_result" => {
            let n: usize = p
                .text
                .trim()
                .parse()
                .map_err(|_| "bad result number".to_string())?;
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(browser::open_result(n))
            })
            .map_err(|e| e.to_string())?;
            Ok(())
        }
        "speak" => {
            if p.text.trim().is_empty() {
                return Err("nothing to say".into());
            }
            let _ = tts::speak(app, &p.text, lang);
            Ok(())
        }
        "delay" => {
            let ms = p.ms.clamp(0, 30_000) as u64;
            tokio::task::block_in_place(|| {
                std::thread::sleep(std::time::Duration::from_millis(ms))
            });
            Ok(())
        }
        "schedule" => {
            let when = scheduler::When {
                in_secs: (p.in_secs > 0).then_some(p.in_secs),
                at: (!p.at.is_empty()).then(|| p.at.clone()),
                cron: (!p.cron.is_empty()).then(|| p.cron.clone()),
            };
            let acts: Vec<Operation> = p
                .actions
                .iter()
                .cloned()
                .map(Operation::normalize)
                .collect();
            let v = scheduler::add_job(app, &when, p.label.clone(), acts, lang)?;
            // let _ = tts::speak(app, &format!("Scheduled: {}", v.label), lang);
            Ok(())
        }
        "list_schedules" => {
            let n = scheduler::list_jobs().len();
            let msg = if n == 0 {
                "No scheduled tasks.".to_string()
            } else {
                format!("{n} scheduled tasks.")
            };
            let _ = tts::speak(app, &msg, lang);
            Ok(())
        }
        "cancel_schedule" => {
            // job removed already and countdown running -> cancel the countdown
            if scheduler::cancel_by_query(app, &p.text) > 0 || scheduler::cancel_pending(None) {
                Ok(())
            } else {
                Err("no matching scheduled task".into())
            }
        }
        "vision_query" => {
            let img_b64 = helper_functions::capture_screen_base64()?;
            let question = p.text.clone();

            let answer = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async move {
                    llm_provider::query_vision_llm(
                        &reqwest::Client::new(),
                        &img_b64,
                        &question,
                        lang,
                    )
                    .await
                })
            })
            .map_err(|e| e.to_string())?;
            let _ = tts::speak(app, &answer, lang);

            Ok(())
        }
        "contextual_search" => {
            helper_functions::contextual_search(&p.location, &p.text).map_err(|e| e.to_string())?;
            Ok(())
        }
        "browser_tab" => {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(browser::switch_tab(&p.text))
            })
            .map_err(|e| e.to_string())?;
            Ok(())
        }
        "type_text" => fast_match::type_text(&p.text),
        "volume_up" => fast_match::volume_up(),
        "volume_down" => fast_match::volume_down(),
        "mute" => fast_match::mute(),
        "lock_screen" => fast_match::lock_screen(),
        "sleep" => fast_match::sleep_system(),
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
