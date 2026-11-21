//! Optional reference library for the image model.
//!
//! The folder is user supplied: a local directory or a mounted SMB share. The
//! app never mounts it and never copies it into the repository. Subfolders are
//! concepts; the concept with the longest name contained in the vibe wins.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

const EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

pub fn images(root: &Path, vibe: &str, forced_concept: &str) -> Result<Vec<PathBuf>> {
    if root.as_os_str().is_empty() {
        return Ok(Vec::new());
    }
    if !root.is_dir() {
        bail!(
            "reference folder {} is not a directory (set IMAGE_REFERENCES to a mounted folder)",
            root.display()
        );
    }
    let dir = concept_dir(root, vibe, forced_concept);
    let mut files = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_file() && is_image(&path) {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// Rotates through the library so each retry round sees a different subset.
pub fn pick(files: &[PathBuf], offset: usize, count: usize) -> Vec<PathBuf> {
    if files.is_empty() || count == 0 {
        return Vec::new();
    }
    (0..count.min(files.len()))
        .map(|index| files[(offset + index) % files.len()].clone())
        .collect()
}

fn concept_dir(root: &Path, vibe: &str, forced: &str) -> PathBuf {
    let forced = forced.trim();
    if !forced.is_empty() {
        let path = root.join(forced);
        if path.is_dir() {
            return path;
        }
    }
    let vibe_key = normalize(vibe);
    let mut best: Option<(usize, PathBuf)> = None;
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let key = normalize(name);
            if key.is_empty() || !vibe_key.contains(&key) {
                continue;
            }
            if best.as_ref().is_none_or(|(length, _)| key.len() > *length) {
                best = Some((key.len(), path));
            }
        }
    }
    best.map(|(_, path)| path)
        .unwrap_or_else(|| root.to_path_buf())
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .is_some_and(|extension| EXTENSIONS.contains(&extension.as_str()))
}

fn normalize(text: &str) -> String {
    text.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
#[path = "tests/references.rs"]
mod tests;
