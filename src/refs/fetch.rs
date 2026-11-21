//! The fetch flow: ask a source for candidates, download them, normalize what
//! comes from a pack, deduplicate and append to the manifest.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use super::store::{Entry, Library, Stored, WriteRequest, resolve_root};
use super::util::{self, ExtractRequest, HttpRequest, Scratch};
use super::{Candidate, Kind, matches_tags};
use crate::config::Config;

pub struct Options {
    pub source: String,
    pub concepts: Vec<String>,
    pub query: Option<String>,
    pub count: usize,
    pub out: Option<PathBuf>,
    pub tags: Vec<String>,
    pub max_files_per_pack: usize,
    pub max_bytes: u64,
    pub max_pack_bytes: u64,
    pub size: u32,
    pub delay: Duration,
    pub dry_run: bool,
    pub packs: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            source: "pixabay".to_string(),
            concepts: Vec::new(),
            query: None,
            count: 40,
            out: None,
            tags: Vec::new(),
            max_files_per_pack: 60,
            max_bytes: 6_000_000,
            max_pack_bytes: 40_000_000,
            size: 768,
            delay: Duration::from_millis(200),
            dry_run: false,
            packs: false,
        }
    }
}

#[derive(Default, Clone)]
struct Counter {
    added: usize,
    skipped: usize,
    filtered: usize,
    failed: usize,
    planned: usize,
}

impl Counter {
    fn reached(&self, count: usize) -> bool {
        self.added >= count || self.planned >= count
    }

    fn merge(&mut self, other: Self) {
        self.added += other.added;
        self.skipped += other.skipped;
        self.filtered += other.filtered;
        self.failed += other.failed;
        self.planned += other.planned;
    }
}

struct Job<'a> {
    client: &'a reqwest::blocking::Client,
    library: &'a mut Library,
    options: &'a Options,
    source: &'a str,
    concept: &'a str,
    query: &'a str,
    stamp: &'a str,
}

impl Job<'_> {
    fn image(&mut self, candidate: &Candidate, counter: &mut Counter) -> Result<()> {
        let raw_extension = util::url_extension(&candidate.url);
        let extension = if matches!(raw_extension.as_str(), "png" | "jpg" | "jpeg" | "webp") {
            raw_extension.clone()
        } else {
            "png".to_string()
        };
        if self.options.dry_run {
            println!(
                "   would fetch {}/{}-<hash>.{}  {}",
                self.concept, self.source, extension, candidate.page_url
            );
            counter.planned += 1;
            return Ok(());
        }
        let label = if candidate.page_url.is_empty() {
            candidate.url.as_str()
        } else {
            candidate.page_url.as_str()
        };
        let downloaded = match util::http_get(HttpRequest {
            client: self.client,
            url: &candidate.url,
            label,
            max_bytes: self.options.max_bytes,
        }) {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("warning: {error}");
                counter.failed += 1;
                return Ok(());
            }
        };
        if downloaded.len() < 5000 {
            counter.failed += 1;
            return Ok(());
        }
        let (bytes, extension) = if extension == "png" && raw_extension != "png" {
            let scratch = Scratch::new("image")?;
            let source = scratch.path().join("input");
            fs::write(&source, &downloaded)
                .with_context(|| format!("writing {}", source.display()))?;
            let normalized = scratch.path().join("normalized.png");
            if let Err(error) = util::normalize_png(&source, &normalized, self.options.size) {
                eprintln!("warning: {error}");
                counter.failed += 1;
                return Ok(());
            }
            let bytes = fs::read(&normalized)
                .with_context(|| format!("reading {}", normalized.display()))?;
            (bytes, extension)
        } else {
            (downloaded, extension)
        };
        self.store(candidate, &bytes, &extension, counter)
    }

    fn pack(&mut self, candidate: &Candidate, counter: &mut Counter) -> Result<()> {
        if self.options.dry_run {
            println!(
                "   would fetch {}/*.png from the zip  {}",
                self.concept, candidate.page_url
            );
            counter.planned += 1;
            return Ok(());
        }
        let limit = self
            .options
            .max_files_per_pack
            .min(self.options.count.saturating_sub(counter.added));
        if limit == 0 {
            return Ok(());
        }
        let label = if candidate.page_url.is_empty() {
            candidate.url.as_str()
        } else {
            candidate.page_url.as_str()
        };
        let bytes = match util::http_get(HttpRequest {
            client: self.client,
            url: &candidate.url,
            label,
            max_bytes: self.options.max_pack_bytes,
        }) {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("warning: {error}");
                counter.failed += 1;
                return Ok(());
            }
        };
        let scratch = Scratch::new("pack")?;
        let archive = scratch.path().join("pack.zip");
        fs::write(&archive, &bytes).with_context(|| format!("writing {}", archive.display()))?;
        let members = match util::extract_images(ExtractRequest {
            archive: &archive,
            dest: &scratch.path().join("images"),
            limit: limit.saturating_mul(4),
            max_bytes: self.options.max_pack_bytes.saturating_mul(20),
        }) {
            Ok(members) => members,
            Err(error) => {
                eprintln!("warning: {error}");
                counter.failed += 1;
                return Ok(());
            }
        };
        let mut added_from_pack = 0;
        for member in members {
            if added_from_pack >= limit {
                break;
            }
            let normalized = scratch
                .path()
                .join(format!("normalized-{added_from_pack}.png"));
            if let Err(error) = util::normalize_png(&member.path, &normalized, self.options.size) {
                eprintln!("warning: {error}");
                counter.failed += 1;
                continue;
            }
            let bytes = fs::read(&normalized)
                .with_context(|| format!("reading {}", normalized.display()))?;
            let mut entry = self.entry(candidate);
            entry.archive = candidate.url.clone();
            entry.original = member.name.clone();
            let written = self.write(&bytes, "png", entry)?;
            match written {
                Stored::Added(path) => {
                    counter.added += 1;
                    added_from_pack += 1;
                    println!("   + {}  ({})", self.display(&path), member.name);
                }
                Stored::Skipped => counter.skipped += 1,
            }
        }
        Ok(())
    }

    fn store(
        &mut self,
        candidate: &Candidate,
        bytes: &[u8],
        extension: &str,
        counter: &mut Counter,
    ) -> Result<()> {
        let entry = self.entry(candidate);
        match self.write(bytes, extension, entry)? {
            Stored::Added(path) => {
                counter.added += 1;
                println!("   + {}", self.display(&path));
            }
            Stored::Skipped => counter.skipped += 1,
        }
        Ok(())
    }

    fn entry(&self, candidate: &Candidate) -> Entry {
        Entry {
            title: candidate.title.clone(),
            author: candidate.author.clone(),
            license: candidate.license.clone(),
            page_url: candidate.page_url.clone(),
            url: candidate.url.clone(),
            id: candidate.id.clone(),
            tags: candidate.tags.clone(),
            query: self.query.to_string(),
            fetched_at: self.stamp.to_string(),
            ..Entry::new(self.concept, self.source)
        }
    }

    fn write(&mut self, bytes: &[u8], extension: &str, entry: Entry) -> Result<Stored> {
        let hash = util::sha256_bytes(bytes);
        self.library.write(WriteRequest {
            concept: self.concept,
            source: self.source,
            hash: &hash,
            extension,
            bytes,
            entry,
        })
    }

    fn display(&self, path: &Path) -> String {
        path.strip_prefix(self.library.root())
            .unwrap_or(path)
            .display()
            .to_string()
    }
}

pub fn run(cfg: &Config, options: Options) -> Result<()> {
    if options.count == 0 {
        return Ok(());
    }
    if options.size < 64 {
        bail!("--size must be at least 64");
    }
    let source = super::source(cfg, &options.source)?;
    let defaults = source.defaults();
    let concepts = if options.concepts.is_empty() {
        defaults.concepts.clone()
    } else {
        options.concepts.clone()
    };
    if concepts.is_empty() {
        bail!(
            "source '{}' has no default concepts, pass --concept",
            source.name()
        );
    }
    if options.query.is_some() && concepts.len() != 1 {
        bail!("--query needs exactly one --concept");
    }
    for concept in &concepts {
        if !util::valid_concept(concept) {
            bail!("invalid concept name: {concept}");
        }
    }

    let client = util::http_client()?;
    let root = resolve_root(cfg, options.out.as_deref());
    let mut library = Library::open(root)?;
    let stamp = util::now_iso();
    let mut totals = Counter::default();
    let mut summary = Vec::new();

    for concept in &concepts {
        let query = options
            .query
            .clone()
            .or_else(|| defaults.queries.get(concept).cloned())
            .with_context(|| {
                format!(
                    "no default query for '{concept}' from '{}', pass --query",
                    source.name()
                )
            })?;
        println!(
            "== {concept}: \"{query}\" via {} (up to {} files)",
            source.name(),
            options.count
        );
        let margin = options.count + options.count / 2 + 5;
        let candidates = source.candidates(concept, &query, margin)?;
        let mut counter = Counter::default();
        for candidate in candidates {
            if counter.reached(options.count) {
                break;
            }
            if !matches_tags(&candidate, &options.tags) {
                counter.filtered += 1;
                continue;
            }
            let mut job = Job {
                client: &client,
                library: &mut library,
                options: &options,
                source: source.name(),
                concept,
                query: &query,
                stamp: &stamp,
            };
            match candidate.kind {
                Kind::Image => job.image(&candidate, &mut counter)?,
                Kind::Zip if options.packs => job.pack(&candidate, &mut counter)?,
                Kind::Zip => {}
            }
            sleep(options.delay);
        }
        summary.push((concept.clone(), counter));
    }

    println!("== summary");
    for (concept, counter) in &summary {
        println!(
            "   {:<12} {:>3} new {:>3} skipped {:>3} filtered {:>3} failed",
            concept, counter.added, counter.skipped, counter.filtered, counter.failed
        );
        totals.merge(counter.clone());
    }
    if options.dry_run {
        println!(
            "   dry run: nothing written to {}",
            library.root().display()
        );
    } else {
        println!("   library: {}", library.root().display());
        println!("   manifest: {}", library.manifest().display());
    }
    if totals.added == 0 && totals.skipped == 0 && totals.planned == 0 {
        eprintln!("   no images matched; try another query or source");
    }
    Ok(())
}
