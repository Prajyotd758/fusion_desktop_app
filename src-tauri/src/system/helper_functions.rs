use crate::state::{self, TaskStatus};
use crate::system::types::Operation;
use crate::tts;
use crate::ui_callbacks::check_internet;
use base64::{engine::general_purpose, Engine};
use enigo::{Direction::Click, Enigo, Key, Keyboard, Settings};
use once_cell::sync::Lazy;
use screenshots::image::ImageOutputFormat;
use screenshots::Screen;
use std::collections::HashMap;
use std::path::PathBuf;
use std::path::{Component, Path};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use windows::Win32::System::ProcessStatus::GetModuleBaseNameW;
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
};

/// System/protected directories that should never be silently deleted.
/// Matched case-insensitively against the resolved absolute path.
const PROTECTED_DIRS: &[&str] = &[
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "system32",
    "boot",
    "recovery",
    "$recycle.bin",
    "users\\default",
    "users\\public",
    "appdata",
];

/// Flat alias->exe_path index built from app_cache.json (display names + aliases).
pub static DISCOVERED_APPS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

static LAST_FOLDER: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

pub fn get_last_folder() -> Option<PathBuf> {
    LAST_FOLDER.get().and_then(|m| m.lock().unwrap().clone())
}

pub fn desktop_dir() -> PathBuf {
    dirs::desktop_dir().unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Creates a Command with the console window suppressed on Windows,
/// preventing the flash-then-close terminal that appears when spawning
/// console-subsystem processes (powershell, cmd, reg, etc.) from a GUI app.
pub fn silent_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// Builds a compact context string describing what's currently focused,
/// to inject into the LLM prompt so it can resolve "this folder"/"current app"
/// without guessing or relying on stateful fallbacks.
pub fn build_focus_context() -> String {
    let app = get_focused_app()
        .map(|(process, title)| format!("Focused app: {process} (\"{title}\")"))
        .unwrap_or_else(|| "Focused app: none detected".to_string());

    let folder = get_focused_explorer_path()
        .map(|p| format!("Focused Explorer folder: {}", p.display()))
        .unwrap_or_else(|| {
            "Focused Explorer folder: none (no Explorer window focused)".to_string()
        });

    println!("app : {app}, folder : {folder}");

    format!("{app}\n{folder}")
}

pub fn get_focused_explorer_path() -> Option<PathBuf> {
    let output = silent_command("powershell")
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

/// "Smart" default: whatever Explorer folder is currently focused, else Desktop.
pub fn default_workspace() -> PathBuf {
    let focused = get_focused_explorer_path();
    focused.or_else(get_last_folder).unwrap_or_else(desktop_dir)
}

pub fn open_with_shell(path: &PathBuf) -> Result<(), String> {
    let status = silent_command("cmd")
        .args(["/C", "start", "", path.to_str().unwrap_or("")])
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("could not open {}", path.display()))
    }
}

pub fn get_focused_app() -> Option<(String, String)> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }

        // Window title
        let mut title_buf = [0u16; 256];
        let len = GetWindowTextW(hwnd, &mut title_buf);
        let title = String::from_utf16_lossy(&title_buf[..len as usize]);

        // Process name
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let mut process_name = String::from("unknown");
        if let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            let mut buf = [0u16; 260];
            let len = GetModuleBaseNameW(handle, None, &mut buf);
            if len > 0 {
                process_name = String::from_utf16_lossy(&buf[..len as usize]);
            }
        }

        Some((process_name, title))
    }
}

/// Returns true if `path` is the C:\ drive root itself, or falls under it
/// shallowly enough (drive root, or a protected system folder) that
/// deleting/modifying it could break the OS or other user data.
pub fn is_dangerous(path: &Path) -> bool {
    let Ok(canonical) = path.canonicalize() else {
        // Path doesn't exist or can't be resolved — treat unresolved
        // paths as dangerous rather than silently allowing them through.
        return true;
    };

    let path_str = canonical.to_string_lossy().to_lowercase();

    // Block the drive root outright (e.g. "C:\", "D:\").
    let components: Vec<Component> = canonical.components().collect();
    if components.len() <= 1 {
        return true;
    }

    // Block anything under C:\ that's a top-level protected folder,
    // or C:\ itself with nothing but a protected dir as the only child.
    if path_str.starts_with("c:\\") {
        // Only 2 components (C:\ + one folder) AND that folder is protected,
        // OR any protected keyword appears anywhere in the path at all.
        if components.len() == 2 {
            let top_level = canonical
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();

            if !PROTECTED_DIRS.iter().any(|d| top_level == *d) {
                // Non-protected top-level folder directly on C:\ (e.g. C:\Projects)
                // is still risky to nuke via voice — flag it too, be conservative.
                return true;
            }
        }
    }

    PROTECTED_DIRS.iter().any(|dir| {
        path_str.contains(&format!("\\{dir}\\")) || path_str.contains(&format!("\\{dir}"))
    })
}

// value = (launch_kind, target)
#[derive(Debug, Clone, Copy)]
pub enum LaunchKind {
    Exe,
    UriShell,
    Msc,
}

pub static APP_ALIASES: Lazy<HashMap<&'static str, (LaunchKind, &'static str)>> = Lazy::new(|| {
    use LaunchKind::*;
    HashMap::from([
        // Editors / IDEs
        ("cursor", (Exe, "Cursor")),
        ("cursor ide", (Exe, "Cursor")),
        ("vs code", (Exe, "code")),
        ("vscode", (Exe, "code")),
        ("visual studio code", (Exe, "code")),
        ("code", (Exe, "code")),
        ("visual studio", (Exe, "devenv.exe")),
        ("sublime", (Exe, "sublime_text.exe")),
        ("sublime text", (Exe, "sublime_text.exe")),
        ("notepad", (Exe, "notepad.exe")),
        ("notepad++", (Exe, "notepad++.exe")),
        ("notepad plus plus", (Exe, "notepad++.exe")),
        ("android studio", (Exe, "studio64.exe")),
        ("intellij", (Exe, "idea64.exe")),
        ("intellij idea", (Exe, "idea64.exe")),
        ("pycharm", (Exe, "pycharm64.exe")),
        // Browsers
        ("chrome", (Exe, "chrome.exe")),
        ("google chrome", (Exe, "chrome.exe")),
        ("firefox", (Exe, "firefox.exe")),
        ("mozilla", (Exe, "firefox.exe")),
        ("edge", (Exe, "msedge.exe")),
        ("microsoft edge", (Exe, "msedge.exe")),
        ("brave", (Exe, "brave.exe")),
        // Office
        ("word", (Exe, "winword.exe")),
        ("microsoft word", (Exe, "winword.exe")),
        ("excel", (Exe, "excel.exe")),
        ("microsoft excel", (Exe, "excel.exe")),
        ("powerpoint", (Exe, "powerpnt.exe")),
        ("microsoft powerpoint", (Exe, "powerpnt.exe")),
        ("outlook", (Exe, "outlook.exe")),
        // Comms
        ("whatsapp", (Exe, "whatsapp.exe")),
        ("telegram", (Exe, "telegram.exe")),
        ("discord", (Exe, "discord.exe")),
        ("slack", (Exe, "slack.exe")),
        ("zoom", (Exe, "zoom.exe")),
        ("teams", (Exe, "teams.exe")),
        ("microsoft teams", (Exe, "teams.exe")),
        ("skype", (Exe, "skype.exe")),
        // Media
        ("spotify", (Exe, "spotify.exe")),
        ("vlc", (Exe, "vlc.exe")),
        ("vlc media player", (Exe, "vlc.exe")),
        ("media player", (Exe, "wmplayer.exe")),
        ("obs", (Exe, "obs64.exe")),
        ("obs studio", (Exe, "obs64.exe")),
        ("photoshop", (Exe, "photoshop.exe")),
        ("adobe photoshop", (Exe, "photoshop.exe")),
        ("premiere", (Exe, "premiere pro.exe")),
        ("premiere pro", (Exe, "premiere pro.exe")),
        // Dev tools
        ("terminal", (Exe, "wt.exe")),
        ("windows terminal", (Exe, "wt.exe")),
        ("cmd", (Exe, "cmd.exe")),
        ("command prompt", (Exe, "cmd.exe")),
        ("powershell", (Exe, "powershell.exe")),
        ("git bash", (Exe, "git-bash.exe")),
        ("docker", (Exe, "Docker Desktop.exe")),
        ("docker desktop", (Exe, "Docker Desktop.exe")),
        ("postman", (Exe, "Postman.exe")),
        ("steam", (Exe, "steam.exe")),
        // System utilities — need special launch handling, not plain spawn
        ("settings", (UriShell, "ms-settings:")),
        ("windows settings", (UriShell, "ms-settings:")),
        ("control panel", (Exe, "control.exe")),
        ("task manager", (Exe, "taskmgr.exe")),
        ("file explorer", (Exe, "explorer.exe")),
        ("explorer", (Exe, "explorer.exe")),
        ("my computer", (Exe, "explorer.exe")),
        ("this pc", (Exe, "explorer.exe")),
        ("device manager", (Msc, "devmgmt.msc")),
        ("disk management", (Msc, "diskmgmt.msc")),
        ("registry editor", (Exe, "regedit.exe")),
        ("regedit", (Exe, "regedit.exe")),
        ("event viewer", (Exe, "eventvwr.exe")),
        ("services", (Msc, "services.msc")),
        ("calculator", (Exe, "calc.exe")),
        ("paint", (Exe, "mspaint.exe")),
        ("ms paint", (Exe, "mspaint.exe")),
        ("snipping tool", (Exe, "SnippingTool.exe")),
    ])
});

pub fn resolve_operation_app_name(mut op: Operation) -> Operation {
    if matches!(
        op.action.as_str(),
        "open_app" | "close_app" | "open_app_in_folder" | "browser_search"
    ) {
        if let Some((_, resolved_name)) = resolve_app(&op.name) {
            op.name = resolved_name.to_string();
        }
    }
    op
}

pub fn init_discovered_apps() {
    DISCOVERED_APPS.set(Mutex::new(HashMap::new())).ok();
}

/// Rebuilds the flat index from an AppCache (call after load or after sync).
pub fn refresh_discovered_apps(cache: &crate::system::types::AppCache) {
    let mut flat = HashMap::new();
    for (name, entry) in &cache.apps {
        flat.insert(name.to_lowercase(), entry.exe_path.clone());
        for alias in &entry.aliases {
            flat.insert(alias.to_lowercase(), entry.exe_path.clone());
        }
    }
    if let Some(map) = DISCOVERED_APPS.get() {
        *map.lock().unwrap() = flat;
    }
}

pub fn resolve_app(input: &str) -> Option<(LaunchKind, String)> {
    let key = input.trim().to_lowercase();

    // hardcoded system tools take priority
    if let Some((kind, target)) = APP_ALIASES.get(key.as_str()) {
        return Some((*kind, target.to_string()));
    }

    // fall back to discovered apps
    if let Some(map) = DISCOVERED_APPS.get() {
        let guard = map.lock().unwrap();
        if let Some(path) = guard.get(&key) {
            return Some((LaunchKind::Exe, path.clone()));
        }
    }

    None
}

pub fn open_resolved(kind: &LaunchKind, target: &str) -> std::io::Result<()> {
    match kind {
        LaunchKind::Exe => {
            // If target is a full resolved path, spawn it directly — no shell needed,
            // no console flash, since it's a native GUI process.
            let path = std::path::Path::new(target);
            if path.is_absolute() && path.exists() {
                silent_command(target).spawn()?;
            } else {
                // fallback: bare name, let cmd/start resolve via PATH or App Paths registry
                silent_command("cmd")
                    .args(["/C", "start", "", target])
                    .spawn()?;
            }
        }
        LaunchKind::UriShell => {
            std::process::Command::new("cmd")
                .args(["/C", "start", "", target])
                .spawn()?;
        }
        LaunchKind::Msc => {
            std::process::Command::new("mmc").arg(target).spawn()?;
        }
    }
    Ok(())
}

pub fn open_browser_search(browser: Option<&str>, query: &str) -> anyhow::Result<()> {
    let url = format!(
        "https://www.google.com/search?q={}",
        urlencoding::encode(query)
    );
    println!("{}", url);
    match browser {
        Some(name) => {
            let exe = match resolve_app(name) {
                Some((_, exe)) => exe,
                None => name.to_string(),
            };
            if silent_command("cmd")
                .args(["/C", "start", "", &exe, &url])
                .spawn()
                .is_err()
            {
                silent_command("cmd")
                    .args(["/C", "start", "", &url])
                    .spawn()?;
            }
            Ok(())
        }
        None => {
            silent_command("cmd")
                .args(["/C", "start", "", &url])
                .spawn()?;
            Ok(())
        }
    }
}

pub fn resolve_browser_exe_path(name: &str) -> Option<String> {
    let exe = match resolve_app(name) {
        Some((_, exe)) => exe,
        None => name.to_string(),
    };
    let exe_with_ext = if exe.to_lowercase().ends_with(".exe") {
        exe.clone()
    } else {
        format!("{exe}.exe")
    };

    let output = std::process::Command::new("reg")
        .args([
            "query",
            &format!(r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{exe_with_ext}"),
            "/ve",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find(|l| l.contains("REG_SZ"))
        .and_then(|l| l.split("REG_SZ").nth(1))
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
}

/// Search input, routed by what's currently focused:
/// - Desktop/unknown -> Windows Search (Win key + type + enter)
/// - Browser focused -> new tab with Google search results
// context_search.rs

pub fn contextual_search(target: &str, query: &str) -> anyhow::Result<()> {
    match target.to_lowercase().as_str() {
        "browser" => browser_search(query),
        _ => desktop_search(query), // default / "desktop"
    }
}

fn browser_search(query: &str) -> anyhow::Result<()> {
    let mut enigo = Enigo::new(&Settings::default())?;

    enigo.key(Key::Control, Click)?;
    enigo.key(Key::Unicode('l'), Click)?; // Ctrl+L focuses address bar
    std::thread::sleep(std::time::Duration::from_millis(200));

    enigo.text(query)?;
    std::thread::sleep(std::time::Duration::from_millis(1000));

    enigo.key(Key::Return, Click)?;
    Ok(())
}

fn desktop_search(query: &str) -> anyhow::Result<()> {
    let mut enigo = Enigo::new(&Settings::default())?;

    enigo.key(Key::Meta, Click)?;
    std::thread::sleep(std::time::Duration::from_millis(300));

    enigo.text(query)?;
    std::thread::sleep(std::time::Duration::from_millis(150));

    enigo.key(Key::Return, Click)?;
    Ok(())
}

pub fn capture_screen_base64() -> Result<String, String> {
    let screens = Screen::all().map_err(|e| e.to_string())?;
    let screen = screens.first().ok_or("no screen found")?;
    let image = screen.capture().map_err(|e| e.to_string())?;

    let mut png_bytes: Vec<u8> = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            ImageOutputFormat::Png,
        )
        .map_err(|e| e.to_string())?;

    Ok(general_purpose::STANDARD.encode(&png_bytes))
}

pub async fn handle_transcribe_failure(
    app: &tauri::AppHandle,
    e: impl std::fmt::Display,
) -> String {
    let err_msg = e.to_string();
    eprintln!("[run_transcribe_only] transcription failed: {err_msg}");

    let online = check_internet().await;
    let spoken = if !online {
        "No internet connection. Please check your network and try again."
    } else {
        "Sorry, I couldn't process that. Please try again."
    };

    let _ = tts::speak(app, spoken, "en");
    state::set_status(TaskStatus::Idle);

    err_msg
}

pub fn groq_key() -> String {
    std::env::var("GROQ_API_KEY").unwrap_or_default()
}
