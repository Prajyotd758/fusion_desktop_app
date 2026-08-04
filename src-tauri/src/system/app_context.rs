use std::path::Path;
use std::process::Command;

/// Launches `app_name` with `folder` as its working directory.
/// VS Code, cmd, and PowerShell each need folder passed differently
/// than a generic GUI app launched via `start`.
pub fn open_app_in_folder(app_name: &str, folder: &Path) -> Result<String, String> {
    if !folder.exists() {
        return Err(format!("folder not found: {}", folder.display()));
    }

    let normalized = app_name.trim().to_lowercase();
    let folder_str = folder.to_str().unwrap_or(".");

    let status = match normalized.as_str() {
        "vscode" | "vs code" | "visual studio code" | "code" => Command::new("cmd")
            .args(["/C", "code", "."])
            .current_dir(folder)
            .status(),

        "cmd" | "command prompt" | "terminal" => Command::new("cmd")
            .args(["/C", "start", "cmd", "/K", "cd", "/D", folder_str])
            .status(),

        "powershell" | "power shell" => Command::new("cmd")
            .args([
                "/C",
                "start",
                "powershell",
                "-NoExit",
                "-Command",
                &format!("Set-Location -LiteralPath '{}'", folder.display()),
            ])
            .status(),

        "windows terminal" | "wt" => Command::new("cmd")
            .args(["/C", "wt", "-d", folder_str])
            .status(),

        // Generic fallback for other GUI apps that respect cwd on launch
        other => Command::new("cmd")
            .args(["/C", "start", "", other])
            .current_dir(folder)
            .status(),
    };

    match status {
        Ok(s) if s.success() => Ok(format!("Opened {app_name} in {}", folder.display())),
        Ok(_) => Err(format!("could not open {app_name} in that folder")),
        Err(e) => Err(e.to_string()),
    }
}


// and one more thing keep in mind or remind me later if i forget, we need to tell the llm that which application is open as well, for eg lets say, user says open random app and open new tab/close tab/close window, the llm should know what the user is talking about   