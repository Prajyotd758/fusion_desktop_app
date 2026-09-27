use crate::system::helper_functions::{open_resolved, resolve_app};
use chrono::Local;
use enigo::{
    Direction::{Click, Press, Release},
    Enigo, Key, Keyboard, Mouse, Settings,
};
use once_cell::sync::Lazy;
use screenshots::Screen;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::ProcessStatus::GetModuleBaseNameW;
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsWindowVisible, ShowWindow, SW_SHOWMINIMIZED,
};

#[derive(Debug)]
pub enum ExecError {
    Enigo(String),
    Io(String),
    UnknownAction(String),
    AppNotFound(String),
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::Enigo(e) => write!(f, "input error: {e}"),
            ExecError::Io(e) => write!(f, "io error: {e}"),
            ExecError::UnknownAction(a) => write!(f, "unknown action: {a}"),
            ExecError::AppNotFound(a) => write!(f, "app not found: {a}"),
        }
    }
}

/// Executes a fast-matched (or LLM-dispatched simple) action.
/// `arg` is only used for `open` (app name) and `type_text` (text to type).
pub fn execute_action(action: &str, arg: Option<&str>) -> Result<(), ExecError> {
    println!(" action : {action}");

    match action {
        "volume_up" => volume_up().map_err(ExecError::Enigo)?,
        "volume_down" => volume_down().map_err(ExecError::Enigo)?,
        "mute" => mute().map_err(ExecError::Enigo)?,

        "lock_screen" => lock_screen().map_err(ExecError::Io)?,
        "sleep" => sleep_system().map_err(ExecError::Io)?,
        "shutdown" => shutdown().map_err(ExecError::Io)?,
        "restart" => restart().map_err(ExecError::Io)?,
        "screenshot" => screenshot().map_err(ExecError::Io)?,

        "copy" => copy().map_err(ExecError::Enigo)?,
        "paste" => paste().map_err(ExecError::Enigo)?,
        "undo" => undo().map_err(ExecError::Enigo)?,
        "redo" => redo().map_err(ExecError::Enigo)?,
        "select_all" => select_all().map_err(ExecError::Enigo)?,
        "save" => save().map_err(ExecError::Enigo)?,

        "media_play_pause" => media_play_pause().map_err(ExecError::Enigo)?,
        "media_next" => media_next().map_err(ExecError::Enigo)?,
        "media_prev" => media_prev().map_err(ExecError::Enigo)?,

        "maximize_window" => maximize_focused_window().map_err(ExecError::Io)?,
        "minimize_window" => match arg {
            Some(name) if !name.is_empty() => minimize_window_by_app(name),
            _ => minimize_focused_window(),
        }
        .map_err(ExecError::Io)?,
        "minimize_all" => minimize_all_windows().map_err(ExecError::Io)?,

        "scroll_up" => scroll_up().map_err(ExecError::Enigo)?,
        "scroll_down" => scroll_down().map_err(ExecError::Enigo)?,

        "switch_tab" => switch_tab().map_err(ExecError::Enigo)?,
        "new_tab" => new_tab().map_err(ExecError::Enigo)?,
        "close_tab" => close_tab().map_err(ExecError::Enigo)?,
        "new_window" => new_window().map_err(ExecError::Enigo)?,
        "go_back" => go_back().map_err(ExecError::Enigo)?,
        "go_forward" => go_forward().map_err(ExecError::Enigo)?,

        "type_text" => {
            let text =
                arg.ok_or_else(|| ExecError::UnknownAction("type_text: missing arg".into()))?;
            type_text(text).map_err(ExecError::Enigo)?;
        }

        "press_key" => {
            let combo =
                arg.ok_or_else(|| ExecError::UnknownAction("press_key: missing arg".into()))?;
            press_keys(combo).map_err(ExecError::Enigo)?;
        }
        "open" => {
            let name = arg.ok_or_else(|| ExecError::UnknownAction("open: missing arg".into()))?;
            let (kind, target) =
                resolve_app(name).ok_or_else(|| ExecError::AppNotFound(name.to_string()))?;
            println!("target : {target}");
            open_resolved(&kind, &target).map_err(|e| ExecError::Io(e.to_string()))?;
        }

        other => return Err(ExecError::UnknownAction(other.to_string())),
    }

    Ok(())
}

static FAST_ACTIONS: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    HashMap::from([
        ("volume up", "volume_up"),
        ("increase volume", "volume_up"),
        ("louder", "volume_up"),
        ("volume down", "volume_down"),
        ("decrease volume", "volume_down"),
        ("quieter", "volume_down"),
        ("mute", "mute"),
        ("unmute", "mute"),
        ("lock screen", "lock_screen"),
        ("lock pc", "lock_screen"),
        ("lock", "lock_screen"),
        ("sleep", "sleep"),
        ("go to sleep", "sleep"),
        ("shutdown", "shutdown"),
        ("shut down", "shutdown"),
        ("power off", "shutdown"),
        ("restart", "restart"),
        ("reboot", "restart"),
        ("screenshot", "screenshot"),
        ("take screenshot", "screenshot"),
        ("take screen shot", "screenshot"),
        ("copy", "copy"),
        ("paste", "paste"),
        ("undo", "undo"),
        ("redo", "redo"),
        ("select all", "select_all"),
        ("save", "save"),
        ("save file", "save"),
        ("play pause", "media_play_pause"),
        ("play", "media_play_pause"),
        ("pause", "media_play_pause"),
        ("next track", "media_next"),
        ("skip", "media_next"),
        ("next song", "media_next"),
        ("previous track", "media_prev"),
        ("last track", "media_prev"),
        ("maximize", "maximize_window"),
        ("maximize window", "maximize_window"),
        ("maximize the window", "maximize_window"),
        ("minimize", "minimize_window"),
        ("minimize window", "minimize_window"),
        ("minimize the window", "minimize_window"),
        ("minimize all", "minimize_all"),
        ("minimize everything", "minimize_all"),
        ("show desktop", "minimize_all"),
        ("scroll up", "scroll_up"),
        ("scroll down", "scroll_down"),
        ("switch tab", "switch_tab"),
        ("next tab", "switch_tab"),
        ("new tab", "new_tab"),
        ("close tab", "close_tab"),
        ("new window", "new_window"),
        ("go back", "go_back"),
        ("back", "go_back"),
        ("go forward", "go_forward"),
        ("forward", "go_forward"),
    ])
});

/// Returns Some(action) only if word_count <= 3 AND it's an exact phrase match.
/// `type_text` is intentionally excluded — it needs an argument, so it always
/// goes through the LLM path.
/// Returns Some((action, optional_arg)).
/// - "type <anything>" → type_text, no word-limit, original casing/punctuation preserved.
/// - "open <app>" → open, app name capped at 3 words (app names are rarely longer).
/// - "minimize <app>" → minimize_window with app name as arg, capped at 3 words.
/// - everything else → exact phrase match in FAST_ACTIONS, capped at 3 words total.
pub fn try_fast_match(transcript: &str) -> Option<(&'static str, Option<String>)> {
    let original = transcript
        .trim()
        .trim_start_matches(|c: char| !c.is_alphanumeric());
    let original = original.trim();
    if original.is_empty() {
        return None;
    }

    let lower = original.to_lowercase();
    if let Some(content) = strip_trigger(original, &lower, "type") {
        if !content.is_empty() {
            return Some(("type_text", Some(content.to_string())));
        }
        return None;
    }
    if let Some(content) = strip_trigger(original, &lower, "dictate") {
        if !content.is_empty() {
            return Some(("type_text", Some(content.to_string())));
        }
        return None;
    }

    // --- "open <app>" — app name capped at 3 words ---
    let cleaned = lower.trim_end_matches(|c: char| !c.is_alphanumeric());
    if let Some(app_name) = cleaned.strip_prefix("open ") {
        let app_name = app_name.trim();
        if !app_name.is_empty() && app_name.split_whitespace().count() <= 3 {
            return Some(("open", Some(app_name.to_string())));
        }
        return None;
    }

    // --- "minimize <app>" — must not match bare "minimize"/"minimize all"/"minimize window",
    // those are handled by the fixed-phrase table below.
    if let Some(app_name) = cleaned.strip_prefix("minimize ") {
        let app_name = app_name.trim();
        let is_generic = matches!(
            app_name,
            "" | "all" | "everything" | "window" | "the window"
        );
        if !is_generic && app_name.split_whitespace().count() <= 3 {
            return Some(("minimize_window", Some(app_name.to_string())));
        }
        // falls through to fixed-phrase table for the generic cases
    }

    // in try_fast_match, before the fixed-phrase table check
    if let Some(key_name) = cleaned.strip_prefix("press ") {
        let key_name = key_name.trim();
        if !key_name.is_empty() {
            return Some(("press_key", Some(key_name.to_string())));
        }
        return None;
    }

    // --- fixed phrase table — whole transcript capped at 3 words ---
    if cleaned.split_whitespace().count() > 3 {
        return None;
    }
    FAST_ACTIONS.get(cleaned).copied().map(|a| (a, None))
}

pub fn maximize_focused_window() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    press_combo(&mut enigo, Key::Meta, Key::UpArrow)
}

pub fn minimize_focused_window() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    // First press restores if maximized, second press minimizes.
    // Pressing twice guarantees minimize regardless of current state.
    press_combo(&mut enigo, Key::Meta, Key::DownArrow)?;
    sleep(Duration::from_millis(80));
    press_combo(&mut enigo, Key::Meta, Key::DownArrow)
}

/// Minimizes all top-level visible windows (like Win+D / "show desktop").
pub fn minimize_all_windows() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    press_combo(&mut enigo, Key::Meta, Key::Unicode('d'))
}

/// Minimizes the first visible top-level window belonging to a process
/// whose name matches `app_name` (case-insensitive, substring match).
pub fn minimize_window_by_app(app_name: &str) -> Result<(), String> {
    let target = app_name.trim().to_lowercase();

    struct SearchCtx {
        target: String,
        found: bool,
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut SearchCtx);

        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1); // continue
        }

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));

        if let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            let mut buf = [0u16; 260];
            let len = GetModuleBaseNameW(handle, None, &mut buf);
            if len > 0 {
                let name = String::from_utf16_lossy(&buf[..len as usize]).to_lowercase();
                if name.contains(&ctx.target) {
                    let _ = ShowWindow(hwnd, SW_SHOWMINIMIZED);
                    ctx.found = true;
                    return BOOL(0); // stop enumeration
                }
            }
        }

        BOOL(1) // continue
    }

    let mut ctx = SearchCtx {
        target,
        found: false,
    };

    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut _ as isize));
    }

    if ctx.found {
        Ok(())
    } else {
        Err(format!("no open window found for {app_name}"))
    }
}

fn strip_trigger<'a>(original: &'a str, lower: &str, trigger: &str) -> Option<&'a str> {
    if !lower.starts_with(trigger) {
        return None;
    }
    let after = &lower[trigger.len()..];
    let after = after.trim_start_matches(['.', ',', ':', ';', '-', '!', '?']);
    if after.is_empty() || after.starts_with(' ') || after.starts_with(char::is_whitespace) {
        let offset = original.len() - after.trim_start().len();
        Some(original[offset..].trim())
    } else {
        None // e.g. "typewriter" shouldn't match "type"
    }
}

fn press_combo(enigo: &mut Enigo, modifier: Key, key: Key) -> Result<(), String> {
    enigo.key(modifier, Press).map_err(|e| e.to_string())?;
    enigo.key(key, Click).map_err(|e| e.to_string())?;
    enigo.key(modifier, Release).map_err(|e| e.to_string())?;
    Ok(())
}

fn tap_key(key: Key) -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    enigo.key(key, Click).map_err(|e| e.to_string())
}

// ---------- system ----------

pub fn volume_up() -> Result<(), String> {
    tap_key(Key::VolumeUp)
}

pub fn volume_down() -> Result<(), String> {
    tap_key(Key::VolumeDown)
}

pub fn mute() -> Result<(), String> {
    tap_key(Key::VolumeMute)
}

pub fn lock_screen() -> Result<(), String> {
    Command::new("rundll32.exe")
        .args(["user32.dll,LockWorkStation"])
        .status()
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn sleep_system() -> Result<(), String> {
    Command::new("rundll32.exe")
        .args(["powrprof.dll,SetSuspendState", "0,1,0"])
        .status()
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn shutdown() -> Result<(), String> {
    Command::new("shutdown")
        .args(["/s", "/t", "0"])
        .status()
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn restart() -> Result<(), String> {
    Command::new("shutdown")
        .args(["/r", "/t", "0"])
        .status()
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn screenshot() -> Result<(), String> {
    let screens = Screen::all().map_err(|e| e.to_string())?;
    let screen = screens.first().ok_or("no screen found")?;
    let image = screen.capture().map_err(|e| e.to_string())?;

    let dir = dirs::picture_dir()
        .ok_or("could not resolve Pictures dir")?
        .join("Screenshots")
        .join("arceus");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let filename = format!("screenshot_{}.png", Local::now().format("%Y%m%d_%H%M%S"));
    let path: PathBuf = dir.join(filename);

    image.save(&path).map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- clipboard / edit ----------

pub fn copy() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    press_combo(&mut enigo, Key::Control, Key::Unicode('c'))
}

pub fn paste() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    press_combo(&mut enigo, Key::Control, Key::Unicode('v'))
}

pub fn undo() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    press_combo(&mut enigo, Key::Control, Key::Unicode('z'))
}

pub fn redo() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    press_combo(&mut enigo, Key::Control, Key::Unicode('y'))
}

pub fn select_all() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    press_combo(&mut enigo, Key::Control, Key::Unicode('a'))
}

pub fn save() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    press_combo(&mut enigo, Key::Control, Key::Unicode('s'))
}

// ---------- media ----------

pub fn media_play_pause() -> Result<(), String> {
    tap_key(Key::MediaPlayPause)
}

pub fn media_next() -> Result<(), String> {
    tap_key(Key::MediaNextTrack)
}

pub fn media_prev() -> Result<(), String> {
    tap_key(Key::MediaPrevTrack)
}

// ---------- scroll / nav ----------

pub fn scroll_up() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    enigo
        .scroll(-3, enigo::Axis::Vertical)
        .map_err(|e| e.to_string())
}

pub fn scroll_down() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    enigo
        .scroll(3, enigo::Axis::Vertical)
        .map_err(|e| e.to_string())
}

pub fn go_back() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    press_combo(&mut enigo, Key::Alt, Key::LeftArrow)
}

pub fn go_forward() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    press_combo(&mut enigo, Key::Alt, Key::RightArrow)
}

// ---------- browser ----------

pub fn new_tab() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    press_combo(&mut enigo, Key::Control, Key::Unicode('t'))
}

pub fn close_tab() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    press_combo(&mut enigo, Key::Control, Key::Unicode('w'))
}

pub fn switch_tab() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    press_combo(&mut enigo, Key::Control, Key::Tab)
}

pub fn new_window() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    press_combo(&mut enigo, Key::Control, Key::Unicode('n'))
}

// ---------- text ----------

pub fn type_text(content: &str) -> Result<(), String> {
    if content.is_empty() {
        return Err("nothing to type".into());
    }
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    enigo.text(content).map_err(|e| e.to_string())
}

// ---------- generic key combo (online LLM path only) ----------

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

pub fn press_keys(combo: &str) -> Result<(), String> {
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
