//! Reference library builder: fetch from a source adapter into concept
//! folders, or import files and packs you already have.
//!
//! The library is only a folder, local or mounted. A source only discovers
//! candidates; download, normalization, deduplication and the manifest live
//! here. Adding a site means one more module implementing [`Source`].

pub mod cli;
pub mod fetch;
pub mod import;
pub mod opengameart;
pub mod pixabay;
pub mod store;
pub mod util;

use std::collections::BTreeMap;

use anyhow::{Result, bail};

use crate::config::Config;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Image,
    Zip,
}

#[derive(Debug, Clone, Default)]
pub struct Candidate {
    pub url: String,
    pub kind: Kind,
    pub title: String,
    pub author: String,
    pub license: String,
    pub page_url: String,
    pub tags: String,
    pub id: String,
}

#[derive(Debug, Clone, Default)]
pub struct Defaults {
    pub concepts: Vec<String>,
    pub queries: BTreeMap<String, String>,
}

pub trait Source {
    fn name(&self) -> &'static str;
    fn defaults(&self) -> Defaults;
    fn candidates(&self, query: &str, count: usize) -> Result<Vec<Candidate>>;
}

pub fn source(cfg: &Config, name: &str) -> Result<Box<dyn Source>> {
    match name {
        "pixabay" => Ok(Box::new(pixabay::Pixabay::from_config(cfg)?)),
        "opengameart" => Ok(Box::new(opengameart::OpenGameArt::new()?)),
        other => bail!("unknown source '{other}': try pixabay or opengameart"),
    }
}

pub fn matches_tags(candidate: &Candidate, tags: &[String]) -> bool {
    if tags.is_empty() {
        return true;
    }
    let haystack = candidate.tags.to_ascii_lowercase();
    tags.iter()
        .any(|tag| haystack.contains(&tag.to_ascii_lowercase()))
}

pub fn defaults(entries: &[(&str, &str)]) -> Defaults {
    let mut result = Defaults::default();
    for (concept, query) in entries {
        result.concepts.push((*concept).to_string());
        result
            .queries
            .insert((*concept).to_string(), (*query).to_string());
    }
    result
}
