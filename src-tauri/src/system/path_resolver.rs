use crate::system::helper_functions::desktop_dir;
use std::path::PathBuf;

pub fn sanitize_name(raw: &str) -> String {
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

fn is_absolute_path(s: &str) -> bool {
    s.len() >= 2 && s.as_bytes()[1] == b':'
}

/// Strips a leading "desktop" segment (any case), since desktop_dir()
/// already represents that root — keeps the remaining subpath as-is.
fn strip_desktop_prefix(location: &str) -> String {
    let segments: Vec<&str> = location
        .split(|c| c == '\\' || c == '/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    if segments.is_empty() {
        return String::new();
    }

    let rest = if segments[0].eq_ignore_ascii_case("desktop") {
        &segments[1..]
    } else {
        &segments[..]
    };

    rest.join("\\")
}

/// Resolves any raw location/source/destination string from the LLM
/// into an absolute path. Absolute paths (C:\...) pass through untouched;
/// everything else is treated as relative to Desktop.
pub fn resolve_full_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();

    if trimmed.is_empty() {
        return desktop_dir();
    }

    if is_absolute_path(trimmed) {
        return PathBuf::from(trimmed);
    }

    let stripped = strip_desktop_prefix(trimmed);
    if stripped.is_empty() {
        desktop_dir()
    } else {
        desktop_dir().join(stripped)
    }
}

/// Base folder for create_folder/create_file (location only, no name joined).
pub fn resolve_base(location: &str) -> PathBuf {
    resolve_full_path(location)
}

/// Full target path for an existing item: location (folder) + name (item).
// pub fn resolve_target(location: &str, name: &str) -> PathBuf {
//     let base = resolve_full_path(location);
//     if name.is_empty() {
//         base
//     } else {
//         base.join(sanitize_name(name))
//     }
// }

pub fn resolve_target(location: &str, name: &str, extension: &str) -> PathBuf {
    let base = resolve_full_path(location);
    if name.is_empty() {
        return base;
    }
    let mut full_name = sanitize_name(name);
    if !extension.is_empty() {
        let ext = extension.trim_start_matches('.');
        if !full_name
            .to_lowercase()
            .ends_with(&format!(".{}", ext.to_lowercase()))
        {
            full_name = format!("{full_name}.{ext}");
        }
    }
    base.join(full_name)
}
