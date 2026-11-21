//! Reference library builder: fetch from a source adapter into concept
//! folders, or import files and packs you already have.
//!
//! The library is only a folder, local or mounted. A source only discovers
//! candidates; download, normalization, deduplication and the manifest live
//! here. Adding a site means one more module implementing [`Source`].

pub mod cli;
pub mod fetch;
pub mod generate;
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

/// One concept folder and how each source searches for it.
#[derive(Debug, Clone, Copy)]
pub struct Concept {
    pub name: &'static str,
    pub term: &'static str,
    /// Overrides `pixel art <term>` on Pixabay.
    pub pixabay: Option<&'static str>,
    /// Overrides `<term>` on OpenGameArt.
    pub opengameart: Option<&'static str>,
}

impl Concept {
    const fn new(name: &'static str, term: &'static str) -> Self {
        Self {
            name,
            term,
            pixabay: None,
            opengameart: None,
        }
    }

    const fn with_pixabay(mut self, query: &'static str) -> Self {
        self.pixabay = Some(query);
        self
    }

    const fn with_opengameart(mut self, query: &'static str) -> Self {
        self.opengameart = Some(query);
        self
    }
}

/// The concept folders the library ships with. The folder name is matched
/// against the vibe, so keep it a single word the vibes actually use.
pub const CONCEPTS: &[Concept] = &[
    Concept::new("battle", "knight"),
    Concept::new("forest", "forest").with_pixabay("pixel art forest background"),
    Concept::new("castle", "castle"),
    Concept::new("jrpg", "rpg")
        .with_pixabay("pixel art game")
        .with_opengameart("rpg icons"),
    Concept::new("boss", "boss"),
    Concept::new("dungeon", "dungeon"),
    Concept::new("cave", "cave"),
    Concept::new("village", "village"),
    Concept::new("desert", "desert"),
    Concept::new("snow", "snow"),
    Concept::new("volcano", "volcano"),
    Concept::new("space", "space"),
    Concept::new("sky", "sky"),
    Concept::new("underwater", "underwater"),
    Concept::new("swamp", "swamp"),
    Concept::new("graveyard", "graveyard"),
    Concept::new("temple", "temple"),
    Concept::new("city", "city"),
];

pub fn concepts_for<F>(query: F) -> Defaults
where
    F: Fn(&Concept) -> String,
{
    let mut result = Defaults::default();
    for concept in CONCEPTS {
        result.concepts.push(concept.name.to_string());
        result
            .queries
            .insert(concept.name.to_string(), query(concept));
    }
    result
}

pub fn pixabay_query(concept: &Concept) -> String {
    concept
        .pixabay
        .map(str::to_string)
        .unwrap_or_else(|| format!("pixel art {}", concept.term))
}

pub fn opengameart_query(concept: &Concept) -> String {
    concept.opengameart.unwrap_or(concept.term).to_string()
}

pub trait Source {
    fn name(&self) -> &'static str;
    fn defaults(&self) -> Defaults;
    fn candidates(&self, concept: &str, query: &str, count: usize) -> Result<Vec<Candidate>>;
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

#[cfg(test)]
#[path = "../tests/refs_concepts.rs"]
mod tests;
