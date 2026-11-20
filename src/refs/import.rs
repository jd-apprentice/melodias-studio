//! The import flow: files, folders and zip packs you already have, normalized
//! and deduplicated into the same library manifest.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::store::{Entry, Library, Stored, WriteRequest, resolve_root};
use super::util::{self, ExtractRequest, Scratch};
use crate::config::Config;

pub struct Options {
    pub concept: String,
    pub license: String,
    pub author: String,
    pub source_url: String,
    pub out: Option<PathBuf>,
    pub size: u32,
    pub prefix: String,
    pub dry_run: bool,
    pub sources: Vec<PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            concept: String::new(),
            license: String::new(),
            author: String::new(),
            source_url: String::new(),
            out: None,
            size: 768,
            prefix: "import".to_string(),
            dry_run: false,
            sources: Vec::new(),
        }
    }
}

pub fn run(cfg: &Config, options: Options) -> Result<()> {
    if options.concept.is_empty() {
        bail!("missing --concept");
    }
    if !util::valid_concept(&options.concept) {
        bail!("invalid concept name: {}", options.concept);
    }
    if options.license.is_empty() {
        bail!("missing --license (for example --license CC0-1.0)");
    }
    if !util::valid_prefix(&options.prefix) {
        bail!("invalid prefix: {}", options.prefix);
    }
    if options.size < 64 {
        bail!("--size must be at least 64");
    }
    if options.sources.is_empty() {
        bail!("missing SOURCE (a file, a folder or a .zip)");
    }

    let root = resolve_root(cfg, options.out.as_deref());
    let mut library = Library::open(root)?;
    let stamp = util::now_iso();
    let scratch = Scratch::new("import")?;
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    let mut failed = 0;
    for source in &options.sources {
        if !source.exists() {
            eprintln!("warning: no such path: {}", source.display());
            failed += 1;
            continue;
        }
        if let Err(error) = collect(source, scratch.path(), &mut files) {
            eprintln!("warning: {error}");
            failed += 1;
        }
    }

    let mut imported = 0;
    let mut skipped = 0;
    let mut seen = 0;
    for (index, (path, original)) in files.iter().enumerate() {
        seen += 1;
        let normalized = scratch.path().join(format!("import-{index}.png"));
        if let Err(error) = util::normalize_png(path, &normalized, options.size) {
            eprintln!("warning: {error}");
            failed += 1;
            continue;
        }
        let bytes =
            fs::read(&normalized).with_context(|| format!("reading {}", normalized.display()))?;
        if bytes.len() < 200 {
            failed += 1;
            continue;
        }
        let hash = util::sha256_bytes(&bytes);
        if options.dry_run {
            if library.contains(&hash) {
                skipped += 1;
                continue;
            }
            println!(
                "would import {} -> {}/{}-{}.png",
                path.display(),
                options.concept,
                options.prefix,
                &hash[..8]
            );
            imported += 1;
            continue;
        }
        let entry = Entry {
            license: options.license.clone(),
            author: options.author.clone(),
            source_url: options.source_url.clone(),
            original: original.clone(),
            imported_at: stamp.clone(),
            ..Entry::new(&options.concept, "import")
        };
        match library.write(WriteRequest {
            concept: &options.concept,
            source: &options.prefix,
            hash: &hash,
            extension: "png",
            bytes: &bytes,
            entry,
        })? {
            Stored::Added(_) => imported += 1,
            Stored::Skipped => skipped += 1,
        }
    }

    if options.dry_run {
        println!("dry run: {imported} would be imported, {failed} failed, nothing written");
        return Ok(());
    }
    println!("imported {imported}, skipped {skipped} duplicates, {failed} failed");
    println!(
        "library: {}",
        library.root().join(&options.concept).display()
    );
    println!("manifest: {}", library.manifest().display());
    if seen == 0 {
        eprintln!("no images found in the sources");
    }
    Ok(())
}

fn is_zip(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(util::is_image_extension)
}

fn collect(path: &Path, scratch: &Path, files: &mut Vec<(PathBuf, String)>) -> Result<()> {
    let metadata = fs::metadata(path).with_context(|| format!("inspecting {}", path.display()))?;
    if metadata.is_dir() {
        let mut entries: Vec<PathBuf> = fs::read_dir(path)
            .with_context(|| format!("reading {}", path.display()))?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .collect();
        entries.sort();
        for entry in entries {
            let metadata = fs::symlink_metadata(&entry)
                .with_context(|| format!("inspecting {}", entry.display()))?;
            if metadata.is_dir() {
                collect(&entry, scratch, files)?;
            } else if metadata.is_file() && is_image(&entry) {
                files.push((entry.clone(), file_name(&entry)));
            }
        }
        return Ok(());
    }
    if is_zip(path) {
        let destination = scratch.join(format!("unpack-{}", files.len()));
        let members = util::extract_images(ExtractRequest {
            archive: path,
            dest: &destination,
            limit: 10_000,
            max_bytes: 2_000_000_000,
        })?;
        for member in members {
            files.push((member.path, member.name));
        }
        return Ok(());
    }
    if is_image(path) {
        files.push((path.to_path_buf(), file_name(path)));
        return Ok(());
    }
    eprintln!("warning: not an image, folder or zip: {}", path.display());
    Ok(())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}
