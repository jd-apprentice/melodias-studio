//! Command line for `melodias refs`: argument parsing and dispatch.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use super::{fetch, generate, import};
use crate::config::Config;

pub const USAGE: &str = "usage: melodias refs fetch [--source NAME] [--concept NAME] [--query TEXT]
                           [--count N] [--out DIR] [--tag TEXT] [--dry-run]
       melodias refs import --concept NAME --license TEXT [options] SOURCE...
       melodias refs generate [--concept NAME] [--count N] [--out DIR] [--dry-run]

fetch options:
  --source NAME           pixabay or opengameart (default pixabay)
  --concept NAME          concept to fill, repeatable; the source provides
                          the default list and queries
  --query TEXT            search query; only valid with one --concept
  --count N               files per concept (default 40)
  --out DIR               library root; default IMAGE_REFERENCES, then
                          image_references in config.toml, then the data dir
  --tag TEXT              keep only candidates whose tags contain TEXT
  --max-files-per-pack N  files imported from one zip (default 60)
  --max-bytes N           skip an image larger than N bytes (default 6000000)
  --max-pack-bytes N      skip a zip larger than N bytes (default 40000000)
  --size N                longest side for images inside a zip (default 768)
  --delay S               seconds between downloads (default 0.2)
  --packs                 also import the zip pack behind an entry (raw assets)
  --dry-run               list what would be imported, write nothing

import options:
  --concept NAME          destination concept folder (required)
  --license TEXT          license of this batch (required)
  --author TEXT           author or pack name
  --source-url URL        page where the pack came from
  --out DIR               library root
  --size N                longest side after resizing (default 768)
  --prefix NAME           file prefix (default import)
  --dry-run               list what would be imported, write nothing

SOURCE can be an image, a folder (searched recursively) or a .zip pack.

generate options:
  --concept NAME          concept to fill, repeatable; default all of them
  --count N               images per concept (default 3, capped at 50)
  --out DIR               library root
  --dry-run               print the prompts and seeds, generate nothing

generate needs sd-server up, the same one the pipeline uses.";

pub fn run(cfg: &Config, args: &[String]) -> Result<()> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("{USAGE}");
        return Ok(());
    }
    match args.first().map(String::as_str) {
        Some("fetch") => fetch::run(cfg, parse_fetch(&args[1..])?),
        Some("import") => import::run(cfg, parse_import(&args[1..])?),
        Some("generate") => generate::run(cfg, parse_generate(&args[1..])?),
        Some(other) => bail!("unknown refs command: {other}\n{USAGE}"),
        None => {
            println!("{USAGE}");
            Ok(())
        }
    }
}

fn next_value(args: &[String], index: &mut usize, flag: &str) -> Result<String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .with_context(|| format!("missing value for {flag}"))
}

fn number<T>(args: &[String], index: &mut usize, flag: &str) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    let value = next_value(args, index, flag)?;
    value
        .parse()
        .with_context(|| format!("{flag} must be a number"))
}

fn parse_fetch(args: &[String]) -> Result<fetch::Options> {
    let mut options = fetch::Options::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--source" => options.source = next_value(args, &mut index, "--source")?,
            "--concept" => options
                .concepts
                .push(next_value(args, &mut index, "--concept")?),
            "--query" => options.query = Some(next_value(args, &mut index, "--query")?),
            "--count" => options.count = number(args, &mut index, "--count")?,
            "--out" => options.out = Some(PathBuf::from(next_value(args, &mut index, "--out")?)),
            "--tag" => options.tags.push(next_value(args, &mut index, "--tag")?),
            "--max-files-per-pack" => {
                options.max_files_per_pack = number(args, &mut index, "--max-files-per-pack")?;
            }
            "--max-bytes" => options.max_bytes = number(args, &mut index, "--max-bytes")?,
            "--max-pack-bytes" => {
                options.max_pack_bytes = number(args, &mut index, "--max-pack-bytes")?;
            }
            "--size" => options.size = number(args, &mut index, "--size")?,
            "--delay" => {
                let seconds: f64 = number(args, &mut index, "--delay")?;
                if !seconds.is_finite() || seconds < 0.0 {
                    bail!("--delay must be zero or more");
                }
                options.delay = Duration::from_secs_f64(seconds);
            }
            "--packs" => options.packs = true,
            "--dry-run" => options.dry_run = true,
            other if other.starts_with('-') => bail!("unknown option: {other}"),
            other => bail!("unexpected argument: {other}"),
        }
        index += 1;
    }
    Ok(options)
}

fn parse_import(args: &[String]) -> Result<import::Options> {
    let mut options = import::Options::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--concept" => options.concept = next_value(args, &mut index, "--concept")?,
            "--license" => options.license = next_value(args, &mut index, "--license")?,
            "--author" => options.author = next_value(args, &mut index, "--author")?,
            "--source-url" => options.source_url = next_value(args, &mut index, "--source-url")?,
            "--out" => options.out = Some(PathBuf::from(next_value(args, &mut index, "--out")?)),
            "--size" => options.size = number(args, &mut index, "--size")?,
            "--prefix" => options.prefix = next_value(args, &mut index, "--prefix")?,
            "--dry-run" => options.dry_run = true,
            other if other.starts_with('-') => bail!("unknown option: {other}"),
            other => options.sources.push(PathBuf::from(other)),
        }
        index += 1;
    }
    Ok(options)
}

fn parse_generate(args: &[String]) -> Result<generate::Options> {
    let mut options = generate::Options::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--concept" => options
                .concepts
                .push(next_value(args, &mut index, "--concept")?),
            "--count" => options.count = number(args, &mut index, "--count")?,
            "--out" => options.out = Some(PathBuf::from(next_value(args, &mut index, "--out")?)),
            "--dry-run" => options.dry_run = true,
            other if other.starts_with('-') => bail!("unknown option: {other}"),
            other => bail!("unexpected argument: {other}"),
        }
        index += 1;
    }
    Ok(options)
}

#[cfg(test)]
#[path = "../tests/refs_cli.rs"]
mod tests;
