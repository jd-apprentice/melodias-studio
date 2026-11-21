//! Generates reference images with the local sd-server.
//!
//! This is the license-safe way to grow the library: the images come from the
//! same model the pipeline uses, so they are yours and match the channel
//! style. The command does not start or stop the GPU services; it expects
//! sd-server to be up, like `melodias image`.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

use super::CONCEPTS;
use super::store::{Entry, Library, Stored, WriteRequest, resolve_root};
use super::util::{self, Scratch};
use crate::clients;
use crate::config::Config;

const MAX_PER_CONCEPT: usize = 50;
const SIZE: u32 = 768;

pub struct Options {
    pub concepts: Vec<String>,
    pub count: usize,
    pub out: Option<PathBuf>,
    pub dry_run: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            concepts: Vec::new(),
            count: 3,
            out: None,
            dry_run: false,
        }
    }
}

pub fn run(cfg: &Config, options: Options) -> Result<()> {
    if options.count == 0 {
        return Ok(());
    }
    if options.count > MAX_PER_CONCEPT {
        bail!("--count is capped at {MAX_PER_CONCEPT} per concept");
    }
    let concepts = if options.concepts.is_empty() {
        CONCEPTS
            .iter()
            .map(|concept| concept.name.to_string())
            .collect()
    } else {
        options.concepts.clone()
    };
    for concept in &concepts {
        if !util::valid_concept(concept) {
            bail!("invalid concept name: {concept}");
        }
    }
    if !options.dry_run {
        clients::check_sd(cfg).with_context(|| {
            if cfg.sd_start_cmd.is_empty() {
                format!("sd-server is not answering at {}", cfg.sd_url)
            } else {
                format!(
                    "sd-server is not answering at {}; start it with `{}`",
                    cfg.sd_url, cfg.sd_start_cmd
                )
            }
        })?;
    }

    let mut library = Library::open(resolve_root(cfg, options.out.as_deref()))?;
    let scratch = Scratch::new("generate")?;
    let stamp = util::now_iso();
    let negative = format!(
        "{}, photo, 3d render, blurry, text, watermark",
        cfg.image_negative_prompt
    );
    let mut added = 0;
    let mut skipped = 0;
    let mut failed = 0;

    for concept in &concepts {
        let prompt = build_prompt(concept, &cfg.image_style);
        println!("== {concept}: {} image(s)", options.count);
        for index in 0..options.count {
            let seed = cfg
                .image_seed
                .map(|base| base.wrapping_add(index as u32))
                .unwrap_or_else(|| random_seed(index));
            if options.dry_run {
                println!("   would generate {concept}/generated-<hash>.png  seed {seed}");
                continue;
            }
            let bytes = match clients::generate_image(cfg, &prompt, &negative, seed, &[]) {
                Ok(bytes) => bytes,
                Err(error) => {
                    eprintln!("warning: {error}");
                    failed += 1;
                    continue;
                }
            };
            let raw = scratch.path().join(format!("{concept}-{index}.png"));
            fs::write(&raw, &bytes).with_context(|| format!("writing {}", raw.display()))?;
            let normalized = scratch.path().join(format!("{concept}-{index}-{SIZE}.png"));
            if let Err(error) = util::normalize_png(&raw, &normalized, SIZE) {
                eprintln!("warning: {error}");
                failed += 1;
                continue;
            }
            let bytes = fs::read(&normalized)
                .with_context(|| format!("reading {}", normalized.display()))?;
            let hash = util::sha256_bytes(&bytes);
            let entry = Entry {
                title: concept.clone(),
                license: "generated".to_string(),
                author: "melodias-studio".to_string(),
                query: prompt.clone(),
                fetched_at: stamp.clone(),
                ..Entry::new(concept, "generated")
            };
            match library.write(WriteRequest {
                concept,
                source: "generated",
                hash: &hash,
                extension: "png",
                bytes: &bytes,
                entry,
            })? {
                Stored::Added(path) => {
                    added += 1;
                    let shown = path
                        .strip_prefix(library.root())
                        .unwrap_or(&path)
                        .display()
                        .to_string();
                    println!("   + {shown}  (seed {seed})");
                }
                Stored::Skipped => skipped += 1,
            }
        }
    }

    if options.dry_run {
        println!("   dry run: nothing generated");
        return Ok(());
    }
    println!("== summary");
    println!("   {added} new, {skipped} skipped, {failed} failed");
    println!("   library: {}", library.root().display());
    println!("   manifest: {}", library.manifest().display());
    Ok(())
}

fn build_prompt(concept: &str, style: &str) -> String {
    format!(
        "{}, {}, limited palette, hard edges, flat shading, no text, no watermark",
        scene(concept),
        style
    )
}

fn scene(concept: &str) -> String {
    match concept {
        "battle" => "a JRPG battlefield at dusk, two armored knights facing each other".to_string(),
        "forest" => "a JRPG forest clearing with tall trees and a mossy path".to_string(),
        "castle" => "a medieval castle on a hill with banners and stone towers".to_string(),
        "jrpg" => "a JRPG town square with shops, a fountain and villagers".to_string(),
        "boss" => "a JRPG boss arena with a giant demon silhouette and torches".to_string(),
        "dungeon" => "a dungeon corridor with stone walls, torches and chains".to_string(),
        "cave" => "a crystal cave with glowing blue crystals and stalactites".to_string(),
        "village" => "a fantasy village with cottages, a well and a dirt road".to_string(),
        "desert" => "a desert with dunes, cacti and half-buried ruins".to_string(),
        "snow" => "a snowy mountain pass with pine trees and falling snow".to_string(),
        "volcano" => "a volcano with lava rivers, ash clouds and black rocks".to_string(),
        "space" => "a space scene with stars, a nebula and a nearby planet".to_string(),
        "sky" => "a sky with clouds, floating islands and a sunset".to_string(),
        "underwater" => "an underwater reef with coral, bubbles and blue light".to_string(),
        "swamp" => "a swamp with twisted trees, murky water and fireflies".to_string(),
        "graveyard" => "a graveyard at night with tombstones, dead trees and fog".to_string(),
        "temple" => "an ancient temple with stone pillars, vines and golden light".to_string(),
        "city" => "a city street at night with neon signs and rain".to_string(),
        other => format!("a {other} scene"),
    }
}

fn random_seed(index: usize) -> u32 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos())
        .unwrap_or(1);
    nanos.wrapping_add((index as u32).wrapping_mul(7919)).max(1)
}

#[cfg(test)]
#[path = "../tests/refs_generate.rs"]
mod tests;
