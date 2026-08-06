use crate::system::helper_functions::{default_workspace, workspace_candidates};
use crate::system::types::OpParams;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use strsim::normalized_levenshtein;

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

pub fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

pub fn find_best_match(target: &str, dir: &Path) -> Option<PathBuf> {
    let target_norm = normalize(target);
    if target_norm.is_empty() {
        return None;
    }

    let entries = fs::read_dir(dir).ok()?;
    let mut best: Option<(PathBuf, f64)> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let full_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let stem_norm = normalize(&stem);
        let full_norm = normalize(&full_name);

        if stem_norm == target_norm || full_norm == target_norm {
            return Some(path);
        }

        let score = normalized_levenshtein(&target_norm, &stem_norm)
            .max(normalized_levenshtein(&target_norm, &full_norm));

        if best.as_ref().map_or(true, |(_, s)| score > *s) {
            best = Some((path, score));
        }
    }

    best.filter(|(_, score)| *score >= 0.7)
        .map(|(path, _)| path)
}

pub fn resolve_base(p: &OpParams, known: &HashMap<String, PathBuf>) -> PathBuf {
    let candidate = if !p.parent.is_empty() && p.parent.to_lowercase() != "desktop" {
        Some(p.parent.as_str())
    } else if !p.location.is_empty() && p.location.to_lowercase() != "desktop" {
        Some(p.location.as_str())
    } else {
        None
    };

    match candidate {
        Some(c) => resolve_named(c, known),
        None => default_workspace(),
    }
}

pub fn resolve_target(p: &OpParams, known: &HashMap<String, PathBuf>) -> PathBuf {
    if !p.path.is_empty() {
        return PathBuf::from(&p.path);
    }
    let name = if !p.location.is_empty() && p.location.to_lowercase() != "desktop" {
        p.location.clone()
    } else {
        p.name.clone()
    };
    resolve_named(&name, known)
}

pub fn resolve_named(name: &str, known: &HashMap<String, PathBuf>) -> PathBuf {
    let sanitized = sanitize_name(name);

    if let Some(path) = known.get(&sanitized.to_lowercase()) {
        return path.clone();
    }

    for dir in workspace_candidates() {
        if let Some(matched) = find_best_match(&sanitized, &dir) {
            return matched;
        }
    }

    default_workspace().join(&sanitized)
}
