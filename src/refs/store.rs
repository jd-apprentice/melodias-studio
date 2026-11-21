//! The reference library on disk: concept folders, the `sources.jsonl`
//! manifest and the hash set used for deduplication.

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use super::util;
use crate::config::Config;

/// One line of `sources.jsonl`. Empty optional fields are omitted so the
/// manifest stays close to the one the old scripts wrote.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub concept: String,
    pub file: String,
    pub source: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub author: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub license: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub page_url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source_url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tags: String,
    pub sha256: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub archive: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub original: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub query: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub fetched_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub imported_at: String,
}

impl Entry {
    pub fn new(concept: &str, source: &str) -> Self {
        Self {
            concept: concept.to_string(),
            file: String::new(),
            source: source.to_string(),
            title: String::new(),
            author: String::new(),
            license: String::new(),
            page_url: String::new(),
            source_url: String::new(),
            url: String::new(),
            id: String::new(),
            tags: String::new(),
            sha256: String::new(),
            archive: String::new(),
            original: String::new(),
            query: String::new(),
            fetched_at: String::new(),
            imported_at: String::new(),
        }
    }
}

pub struct Library {
    root: PathBuf,
    manifest: PathBuf,
    hashes: HashSet<String>,
}

pub enum Stored {
    Added(PathBuf),
    Skipped,
}

pub struct WriteRequest<'a> {
    pub concept: &'a str,
    pub source: &'a str,
    pub hash: &'a str,
    pub extension: &'a str,
    pub bytes: &'a [u8],
    pub entry: Entry,
}

impl Library {
    pub fn open(root: PathBuf) -> Result<Self> {
        let manifest = root.join("sources.jsonl");
        let mut hashes = HashSet::new();
        if let Ok(text) = fs::read_to_string(&manifest) {
            for line in text.lines().filter(|line| !line.trim().is_empty()) {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(line)
                    && let Some(hash) = value.get("sha256").and_then(|value| value.as_str())
                    && let Some(file) = value.get("file").and_then(|value| value.as_str())
                {
                    // A file deleted by hand frees its hash, so the same
                    // content can be fetched again if you change your mind.
                    if root.join(file).exists() {
                        hashes.insert(hash.to_string());
                    }
                }
            }
        }
        Ok(Self {
            root,
            manifest,
            hashes,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn manifest(&self) -> &Path {
        &self.manifest
    }

    pub fn contains(&self, hash: &str) -> bool {
        self.hashes.contains(hash)
    }

    pub fn write(&mut self, request: WriteRequest<'_>) -> Result<Stored> {
        if request.hash.len() < 8 {
            bail!("refusing to name a file with a short hash");
        }
        let short = &request.hash[..8];
        let name = format!("{}-{short}.{}", request.source, request.extension);
        let directory = self.root.join(request.concept);
        let target = directory.join(&name);
        if self.contains(request.hash) || target.exists() {
            return Ok(Stored::Skipped);
        }
        fs::create_dir_all(&directory)
            .with_context(|| format!("creating {}", directory.display()))?;
        fs::write(&target, request.bytes)
            .with_context(|| format!("writing {}", target.display()))?;
        let mut entry = request.entry;
        entry.file = format!("{}/{}", request.concept, name);
        entry.sha256 = request.hash.to_string();
        let line = serde_json::to_string(&entry).context("serializing a manifest line")?;
        let mut manifest = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.manifest)
            .with_context(|| format!("opening {}", self.manifest.display()))?;
        writeln!(manifest, "{line}")
            .with_context(|| format!("appending to {}", self.manifest.display()))?;
        self.hashes.insert(request.hash.to_string());
        Ok(Stored::Added(target))
    }
}

pub fn resolve_root(cfg: &Config, explicit: Option<&Path>) -> PathBuf {
    if let Some(path) = explicit {
        return util::expand_tilde(&path.to_string_lossy());
    }
    if !cfg.image_references.as_os_str().is_empty() {
        return cfg.image_references.clone();
    }
    util::default_root()
}

#[cfg(test)]
#[path = "../tests/refs_store.rs"]
mod tests;
