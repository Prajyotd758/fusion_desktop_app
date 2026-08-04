use std::path::PathBuf;
use std::path::{Component, Path};
use std::process::Command;

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
