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

static LAST_FOLDER: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

pub fn set_last_folder(path: PathBuf) {
    let m = LAST_FOLDER.get_or_init(|| Mutex::new(None));
    *m.lock().unwrap() = Some(path);
}

pub fn get_last_folder() -> Option<PathBuf> {
    LAST_FOLDER.get().and_then(|m| m.lock().unwrap().clone())
}

pub fn desktop_dir() -> PathBuf {
    dirs::desktop_dir().unwrap_or_else(|| PathBuf::from("."))
}

#[tauri::command]
pub fn get_focused_explorer_path() -> Option<PathBuf> {
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
    eprintln!("Focused Explorer path detected: {path}");

    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

/// "Smart" default: whatever Explorer folder is currently focused, else Desktop.
pub fn default_workspace() -> PathBuf {
    let focused = get_focused_explorer_path();
    eprintln!("Focused Explorer path detected: {:?}", focused);
    focused.or_else(get_last_folder).unwrap_or_else(desktop_dir)
}

pub fn workspace_candidates() -> Vec<PathBuf> {
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

pub fn open_with_shell(path: &PathBuf) -> Result<(), String> {
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

// in helper_functions.rs or a new explorer.rs
pub fn get_selected_explorer_item() -> Option<PathBuf> {
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
            $sel = $window.Document.SelectedItems()
            if ($sel.Count -gt 0) {
                Write-Output $sel.Item(0).Path
            }
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
