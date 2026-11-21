//! OpenGameArt source: HTML search plus art pages, CC0 only.
//!
//! The search filters by license id 4 (CC0) and 2D art, and every art page is
//! checked again before its files are offered. The site answers a browser user
//! agent and rate limits politely, so pages are spaced out.
//!
//! Each entry offers one representative image: the original attachment when
//! the medium preview points to one, otherwise the preview itself. The first
//! zip is offered as a candidate too; the dispatcher imports it only when the
//! caller asks for packs.

use std::collections::HashSet;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::Url;

use super::util::{self, HttpRequest, decode_entities, find_all, take_until};
use super::{Candidate, Defaults, Kind, Source, concepts_for, opengameart_query};

const SEARCH_URL: &str = "https://opengameart.org/art-search-advanced";
const BASE_URL: &str = "https://opengameart.org";
const FILE_PREFIX: &str = "https://opengameart.org/sites/default/files/";
const MAX_PAGE_BYTES: u64 = 8 * 1024 * 1024;
const SEARCH_DELAY: Duration = Duration::from_millis(600);
const ENTRY_DELAY: Duration = Duration::from_millis(1200);

pub struct OpenGameArt {
    client: reqwest::blocking::Client,
}

impl OpenGameArt {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: util::http_client()?,
        })
    }

    fn text(&self, url: &str) -> Result<String> {
        let bytes = util::http_get(HttpRequest {
            client: &self.client,
            url,
            label: url,
            max_bytes: MAX_PAGE_BYTES,
        })?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn search(&self, query: &str, wanted: usize) -> Result<Vec<String>> {
        let mut slugs = Vec::new();
        let mut seen = HashSet::new();
        for page in 0..5u32 {
            if slugs.len() >= wanted {
                break;
            }
            let page = page.to_string();
            let url = Url::parse_with_params(
                SEARCH_URL,
                &[
                    ("keys", query),
                    ("field_art_licenses_tid[]", "4"),
                    ("field_art_type_tid[]", "9"),
                    ("sort_by", "score"),
                    ("sort_order", "DESC"),
                    ("items_per_page", "24"),
                    ("page", page.as_str()),
                ],
            )
            .context("building the OpenGameArt search URL")?;
            let body = self.text(url.as_str())?;
            let mut found = 0;
            for slug in slugs_in(&body) {
                if slug == "faq" || !seen.insert(slug.clone()) {
                    continue;
                }
                slugs.push(slug);
                found += 1;
            }
            if found == 0 {
                break;
            }
            sleep(SEARCH_DELAY);
        }
        Ok(slugs)
    }

    fn entry(&self, slug: &str, query: &str) -> Result<Vec<Candidate>> {
        let page_url = format!("{BASE_URL}/content/{slug}");
        let body = self.text(&page_url)?;
        let names = licenses_in(&body);
        if !is_cc0(&names) {
            eprintln!(
                "warning: {slug}: not CC0 ({}), skipped",
                if names.is_empty() {
                    "unknown license".to_string()
                } else {
                    names.join(", ")
                }
            );
            return Ok(Vec::new());
        }
        let title = title_in(&body).unwrap_or_else(|| slug.replace('-', " "));
        if !relevant(&title, slug, query) {
            eprintln!("warning: {slug}: title does not match \"{query}\", skipped");
            return Ok(Vec::new());
        }
        let author = author_in(&body).unwrap_or_default();
        let files = files_in(&body);
        let images: Vec<&String> = files
            .iter()
            .filter(|url| util::is_image_extension(&util::url_extension(url)))
            .collect();
        let packs: Vec<&String> = files
            .iter()
            .filter(|url| util::url_extension(url) == "zip")
            .collect();
        let preview = preview_in(&body);
        // The preview is a scaled copy. When the original is among the
        // attachments, use it: previews are often only 250 px wide.
        let image = preview
            .as_deref()
            .and_then(|url| original_for(url, &files))
            .or_else(|| images.first().map(|url| (*url).clone()))
            .or(preview);
        let mut out = Vec::new();
        if let Some(url) = image {
            out.push(Candidate {
                url,
                kind: Kind::Image,
                title: title.clone(),
                author: author.clone(),
                license: "CC0-1.0".to_string(),
                page_url: page_url.clone(),
                tags: "cc0".to_string(),
                id: String::new(),
            });
        }
        if let Some(url) = packs.first() {
            out.push(Candidate {
                url: (*url).clone(),
                kind: Kind::Zip,
                title,
                author,
                license: "CC0-1.0".to_string(),
                page_url,
                tags: "cc0 pack".to_string(),
                id: String::new(),
            });
        }
        Ok(out)
    }
}

impl Source for OpenGameArt {
    fn name(&self) -> &'static str {
        "opengameart"
    }

    fn defaults(&self) -> Defaults {
        concepts_for(opengameart_query)
    }

    fn candidates(&self, _concept: &str, query: &str, count: usize) -> Result<Vec<Candidate>> {
        let wanted = count.clamp(4, 48);
        let mut out = Vec::new();
        for slug in self.search(query, wanted)? {
            if out.len() >= count {
                break;
            }
            match self.entry(&slug, query) {
                Ok(mut found) => out.append(&mut found),
                Err(error) => eprintln!("warning: {slug}: {error}"),
            }
            sleep(ENTRY_DELAY);
        }
        out.truncate(count);
        Ok(out)
    }
}

/// OpenGameArt search matches the description too, so a page can rank high
/// without being about the query. The first query word has to appear in the
/// title or the slug; very short words are ignored.
fn relevant(title: &str, slug: &str, query: &str) -> bool {
    let needle = query.split_whitespace().next().unwrap_or("");
    if needle.chars().count() < 3 {
        return true;
    }
    let needle = needle.to_ascii_lowercase();
    title.to_ascii_lowercase().contains(&needle) || slug.to_ascii_lowercase().contains(&needle)
}

fn slugs_in(html: &str) -> Vec<String> {
    find_all(html, "href=\"/content/")
        .into_iter()
        .map(|rest| take_until(rest, &['"', '#', '?']).to_string())
        .filter(|slug| !slug.is_empty())
        .collect()
}

fn licenses_in(html: &str) -> Vec<String> {
    let mut out: Vec<String> = find_all(html, "license-name'>")
        .into_iter()
        .map(|rest| decode_entities(take_until(rest, &['<'])))
        .filter(|name| !name.is_empty())
        .collect();
    out.dedup();
    out
}

fn title_in(html: &str) -> Option<String> {
    if let Some(rest) = find_all(html, "property=\"dc:title\"><h2>").first() {
        let title = decode_entities(take_until(rest, &['<']));
        if !title.is_empty() {
            return Some(title);
        }
    }
    find_all(html, "<title>").first().map(|rest| {
        decode_entities(take_until(rest, &['<']))
            .replace("| OpenGameArt.org", "")
            .trim()
            .to_string()
    })
}

fn author_in(html: &str) -> Option<String> {
    let blocks = find_all(html, "field-name-author-submitter");
    let block = blocks.first()?;
    let block = &block[..block.len().min(4000)];
    let links = find_all(block, "<a href=\"/users/");
    let rest = links.first()?;
    let start = rest.find("\">")? + 2;
    let name = decode_entities(take_until(&rest[start..], &['<']));
    (!name.is_empty()).then_some(name)
}

fn preview_in(html: &str) -> Option<String> {
    for marker in ["/styles/medium/public/", "/styles/large/public/"] {
        if let Some(rest) = find_all(html, marker).first() {
            let path = take_until(rest, &['"', '\'', ' ', '<', '>', '\n', '\r']);
            if !path.is_empty() {
                return Some(format!("{BASE_URL}/sites/default/files{marker}{path}"));
            }
        }
    }
    None
}

fn is_cc0(licenses: &[String]) -> bool {
    licenses
        .iter()
        .any(|name| name.to_ascii_uppercase().starts_with("CC0"))
}

fn original_for(preview: &str, files: &[String]) -> Option<String> {
    for marker in ["/styles/medium/public/", "/styles/large/public/"] {
        if let Some(index) = preview.find(marker) {
            let candidate = format!("{}/{}", &preview[..index], &preview[index + marker.len()..]);
            if files.iter().any(|url| url == &candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn files_in(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    for rest in find_all(html, FILE_PREFIX) {
        let path = take_until(rest, &['"', '\'', ' ', '<', '>', '\n', '\r']);
        if path.is_empty()
            || path.starts_with("license_images/")
            || path.starts_with("styles/")
            || path.starts_with("pictures/")
            || path.contains("comment_type")
        {
            continue;
        }
        let url = format!("{FILE_PREFIX}{path}");
        if !out.contains(&url) {
            out.push(url);
        }
    }
    out
}

#[cfg(test)]
#[path = "../tests/refs_opengameart.rs"]
mod tests;
