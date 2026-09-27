// src/system/app_discovery.rs
use crate::system::helper_functions::groq_key;
use crate::system::types::{
    AliasResponse, AppCache, AppEntry, GroqRequest, GroqResponse, LlmError, GroqMessage
};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use walkdir::WalkDir; // adjust to your actual fn signature

use std::path::Path;

fn cache_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("app_cache.json")
}

pub fn load_cache(app_data_dir: &Path) -> AppCache {
    match fs::read_to_string(cache_path(app_data_dir)) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => AppCache::default(),
    }
}

pub fn save_cache(app_data_dir: &Path, cache: &AppCache) -> std::io::Result<()> {
    let s = serde_json::to_string_pretty(cache)?;
    fs::write(cache_path(app_data_dir), s)
}

pub async fn sync_app_aliases(app_data_dir: &Path) {
    let scanned = filter_and_dedupe(scan_start_menu());
    let mut cache = load_cache(app_data_dir);
    crate::system::helper_functions::refresh_discovered_apps(&cache);

    let known_paths: std::collections::HashSet<&str> =
        cache.apps.values().map(|e| e.exe_path.as_str()).collect();

    let new_apps: HashMap<String, String> = scanned
        .into_iter()
        .filter(|(_, path)| !known_paths.contains(path.as_str()))
        .collect();

    println!(
        "[app_discovery] {} apps in cache, {} new",
        cache.apps.len(),
        new_apps.len()
    );

    if new_apps.is_empty() {
        return;
    }

    match fetch_aliases(&new_apps).await {
        Ok(alias_map) => {
            for (name, path) in &new_apps {
                let aliases = alias_map.get(path).cloned().unwrap_or_default();
                println!("[app_discovery] {name} -> {path} | aliases: {aliases:?}");
                cache.apps.insert(
                    name.clone(),
                    AppEntry {
                        exe_path: path.clone(),
                        aliases,
                    },
                );
            }
            match save_cache(app_data_dir, &cache) {
                Ok(_) => {
                    println!(
                        "[app_discovery] cache saved, {} total apps",
                        cache.apps.len()
                    );
                    // refresh in-memory index with the newly added entries
                    crate::system::helper_functions::refresh_discovered_apps(&cache);
                }
                Err(e) => eprintln!("[app_discovery] cache save failed: {e}"),
            }
        }
        Err(e) => eprintln!("[app_discovery] alias fetch failed: {e}"),
    }
}

/// Scans Start Menu .lnk shortcuts. Registry App Paths layer added next.
pub fn scan_start_menu() -> HashMap<String, String> {
    let mut found = HashMap::new();
    let roots = [
        dirs::data_dir().map(|d| d.join(r"Microsoft\Windows\Start Menu\Programs")),
        Some(PathBuf::from(
            r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs",
        )),
    ];

    for root in roots.into_iter().flatten() {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(&root).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("lnk") {
                continue;
            }

            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            if name.is_empty() {
                continue;
            }

            if let Ok(shortcut) = lnk::ShellLink::open(path, encoding_rs::UTF_8) {
                if let Some(target) = shortcut
                    .link_info()
                    .as_ref()
                    .and_then(|li| li.local_base_path().clone())
                {
                    found.insert(name, target.to_string());
                }
            }
        }
    }
    found
}

const BLOCKLIST: &[&str] = &[
    "uninstall",
    "readme",
    "release notes",
    "what is new",
    "documentation",
    "manual",
    "manuals",
    "help",
    "check for updates",
    "about ",
    "license",
    "changelog",
    "reload configuration",
    "install additional tools",
    "developer command prompt",
    "developer powershell",
    "native tools command prompt",
    "cross tools command prompt",
    "debuggable package manager",
    "application verifier",
    "windows app cert kit",
    "gpuview help",
    "cmake documentation",
    "language preferences",
    "windows performance",
    "configure java",
];

fn is_launchable(name: &str, path: &str) -> bool {
    let name_lc = name.to_lowercase();
    if BLOCKLIST.iter().any(|kw| name_lc.contains(kw)) {
        return false;
    }
    path.to_lowercase().ends_with(".exe")
}

/// Filters junk + dedupes multiple shortcut names pointing to the same exe.
pub fn filter_and_dedupe(raw: HashMap<String, String>) -> HashMap<String, String> {
    let mut by_path: HashMap<String, String> = HashMap::new();

    for (name, path) in raw {
        if !is_launchable(&name, &path) {
            continue;
        }
        by_path
            .entry(path.clone())
            .and_modify(|existing| {
                if name.len() < existing.len() {
                    *existing = name.clone();
                }
            })
            .or_insert(name);
    }

    // invert back to name -> path
    by_path
        .into_iter()
        .map(|(path, name)| (name, path))
        .collect()
}

pub async fn fetch_aliases(
    new_apps: &HashMap<String, String>,
) -> Result<HashMap<String, Vec<String>>, String> {
    if new_apps.is_empty() {
        return Ok(HashMap::new());
    }

    let listing = new_apps
        .iter()
        .map(|(name, path)| format!("{name} -> {path}"))
        .collect::<Vec<_>>()
        .join("\n");

    let client = reqwest::Client::new();
    let raw = call_groq_aliases(&client, &listing)
        .await
        .map_err(|e| e.to_string())?;

    let cleaned = raw
        .trim()
        .trim_start_matches("```json")
        .trim_end_matches("```");

    let parsed: AliasResponse = serde_json::from_str(cleaned)
        .map_err(|e| format!("alias parse failed: {e} | raw: {cleaned}"))?;

    Ok(parsed
        .apps
        .into_iter()
        .map(|a| (a.exe_path, a.aliases))
        .collect())
}

pub async fn call_groq_aliases(
    client: &reqwest::Client,
    app_listing: &str,
) -> Result<String, LlmError> {
    const ALIAS_SYSTEM_PROMPT: &str = r#"You generate voice-command aliases for Windows apps.
For each app given as "display_name -> exe_path", return every natural way a user
might SAY the app's name out loud (short forms, common misspellings, spoken variants).
Skip aliases identical to the display name unless it's already natural to speak.

Respond ONLY with JSON, no markdown, no preamble, in this exact shape:
{"apps":[{"exe_path":"...","aliases":["...","..."]}]}"#;

    let body = GroqRequest {
        model: "openai/gpt-oss-120b",
        reasoning_effort: "low",
        max_tokens: 2048,
        temperature: 0.0,
        messages: vec![
            GroqMessage {
                role: "system",
                content: ALIAS_SYSTEM_PROMPT,
            },
            GroqMessage {
                role: "user",
                content: app_listing,
            },
        ],
        response_format: serde_json::json!({ "type": "json_object" }),
    };

    let resp = client
        .post("https://api.groq.com/openai/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", groq_key()))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(LlmError::Api(format!("Groq {status}: {text}")));
    }

    let parsed: GroqResponse = resp
        .json()
        .await
        .map_err(|e| LlmError::Parse(e.to_string()))?;

    parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .ok_or_else(|| LlmError::Parse("no choices in Groq response".into()))
}
