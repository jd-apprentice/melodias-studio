//! Pixabay source: a JSON API, one page of 100 hits at a time.
//!
//! The key is read from `PIXABAY_API_KEY`, then `pixabay_api_key` in the
//! configuration, then `$XDG_CONFIG_HOME/melodias-studio/pixabay.key`. It never
//! goes to disk from here and never belongs in the repository.
//!
//! Results use the Pixabay Content License: free use and adaptation, no
//! attribution, no standalone redistribution, and no logos, brands or
//! recognizable people.

use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::Url;
use reqwest::blocking::Client;
use serde::Deserialize;

use super::util::{HttpRequest, http_get};
use super::{Candidate, Defaults, Source, defaults};
use crate::config::Config;

const MIN_WIDTH: u32 = 400;
const PER_PAGE: u32 = 100;
const MAX_PAGES: u32 = 5;

pub struct Pixabay {
    client: Client,
    key: String,
}

#[derive(Deserialize)]
struct Page {
    hits: Vec<Hit>,
}

#[derive(Deserialize)]
struct Hit {
    id: u64,
    #[serde(rename = "webformatURL", default)]
    webformat_url: String,
    #[serde(rename = "largeImageURL", default)]
    large_image_url: String,
    #[serde(rename = "pageURL", default)]
    page_url: String,
    #[serde(default)]
    user: String,
    #[serde(rename = "imageWidth", default)]
    image_width: u32,
    #[serde(default)]
    tags: String,
}

impl Pixabay {
    pub fn from_config(cfg: &Config) -> Result<Self> {
        let key = std::env::var("PIXABAY_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                let configured = cfg.pixabay_api_key.trim();
                (!configured.is_empty()).then(|| configured.to_string())
            })
            .or_else(key_file)
            .context(
                "no Pixabay key: set PIXABAY_API_KEY, set pixabay_api_key in config.toml \
                 or write $XDG_CONFIG_HOME/melodias-studio/pixabay.key",
            )?;
        Ok(Self {
            client: super::util::http_client()?,
            key,
        })
    }

    fn page(&self, query: &str, page: u32) -> Result<Page> {
        let mut url = Url::parse("https://pixabay.com/api/").context("building the Pixabay URL")?;
        url.query_pairs_mut()
            .append_pair("key", &self.key)
            .append_pair("q", query)
            .append_pair("image_type", "illustration")
            .append_pair("safesearch", "true")
            .append_pair("per_page", &PER_PAGE.to_string())
            .append_pair("page", &page.to_string())
            .append_pair("order", "popular");
        let label = format!("pixabay page {page}");
        let bytes = http_get(HttpRequest {
            client: &self.client,
            url: url.as_str(),
            label: &label,
            max_bytes: 8 * 1024 * 1024,
        })?;
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {label}"))
    }
}

impl Source for Pixabay {
    fn name(&self) -> &'static str {
        "pixabay"
    }

    fn defaults(&self) -> Defaults {
        defaults(&[
            ("battle", "pixel art knight"),
            ("forest", "pixel art forest background"),
            ("castle", "pixel art castle"),
            ("jrpg", "pixel art game"),
        ])
    }

    fn candidates(&self, query: &str, count: usize) -> Result<Vec<Candidate>> {
        let mut out = Vec::new();
        for page in 1..=MAX_PAGES {
            if out.len() >= count {
                break;
            }
            let hits = self.page(query, page)?.hits;
            if hits.is_empty() {
                break;
            }
            for hit in hits {
                if out.len() >= count {
                    break;
                }
                if hit.image_width < MIN_WIDTH {
                    continue;
                }
                let url = if hit.webformat_url.is_empty() {
                    hit.large_image_url
                } else {
                    hit.webformat_url
                };
                if url.is_empty() {
                    continue;
                }
                out.push(Candidate {
                    url,
                    kind: super::Kind::Image,
                    title: hit.tags.split(',').next().unwrap_or("").trim().to_string(),
                    author: hit.user,
                    license: "pixabay-content-license".to_string(),
                    page_url: hit.page_url,
                    tags: hit.tags,
                    id: hit.id.to_string(),
                });
            }
            sleep(Duration::from_secs(1));
        }
        Ok(out)
    }
}

fn key_file() -> Option<String> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    let path = base.join("melodias-studio/pixabay.key");
    let key = std::fs::read_to_string(&path).ok()?;
    let key = key.trim().to_string();
    if key.is_empty() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(&path)
            && metadata.permissions().mode() & 0o077 != 0
        {
            eprintln!(
                "warning: {} is readable by other users; run: chmod 600 {}",
                path.display(),
                path.display()
            );
        }
    }
    Some(key)
}
