//! The public notebook collection, hosted on GitHub.
//!
//! `index.json` at the catalog base URL lists every notebook with the
//! SHA-256 of its source. The engine revalidates the index with its ETag,
//! caches it and every verified source on disk, and falls back to the cache
//! — then to a snapshot bundled into the binary — when GitHub is
//! unreachable, so the collection works offline and on first launch.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::params::{self, NotebookParam};
use crate::error::{Error, Result};
use crate::secrets::write_private;

pub const DEFAULT_CATALOG_URL: &str = "https://raw.githubusercontent.com/nzap-labs/nzap-notebooks/main/";
const BUNDLED: &str = include_str!("../../catalog/bundled.json");
const MAX_INDEX_BYTES: usize = 5 * 1024 * 1024;
const MAX_SOURCE_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub params: Vec<NotebookParam>,
    /// Source path relative to the catalog base URL.
    pub source: String,
    /// Lower-case hex SHA-256 of the source.
    pub sha256: String,
    /// Only in the bundled snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatalogIndex {
    pub version: u32,
    #[serde(default)]
    pub notebooks: Vec<CatalogEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogOrigin {
    /// Freshly fetched (or revalidated) from the catalog URL.
    Remote,
    /// The last good copy on disk.
    Cache,
    /// The snapshot shipped inside the app.
    Bundled,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogStatus {
    pub origin: CatalogOrigin,
    pub url: String,
    pub fetched_at: Option<String>,
    pub error: Option<String>,
    pub count: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CacheMeta {
    url: String,
    etag: Option<String>,
    fetched_at: Option<String>,
}

struct Loaded {
    index: CatalogIndex,
    origin: CatalogOrigin,
    fetched_at: Option<String>,
    error: Option<String>,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_slug(slug: &str) -> bool {
    let mut chars = slug.chars();
    (2..=63).contains(&slug.len())
        && chars.next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn safe_source_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains("://")
        && path.ends_with(".py")
        && path.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

/// Reject anything the engine should not trust from the network.
pub fn validate_index(index: &CatalogIndex) -> Result<()> {
    if index.version != 1 {
        return Err(Error::invalid(format!("Unsupported catalog version {}.", index.version)));
    }
    let mut slugs = std::collections::HashSet::new();
    for entry in &index.notebooks {
        let bad = |reason: &str| Error::invalid(format!("Catalog entry '{}': {reason}", entry.slug));
        if !valid_slug(&entry.slug) || !slugs.insert(entry.slug.as_str()) {
            return Err(bad("invalid or duplicate slug"));
        }
        if entry.title.trim().is_empty() {
            return Err(bad("missing title"));
        }
        if entry.sha256.len() != 64 || !entry.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) {
            return Err(bad("sha256 must be 64 lower-case hex characters"));
        }
        if !safe_source_path(&entry.source) {
            return Err(bad("unsafe source path"));
        }
        params::validate(&entry.params).map_err(|error| bad(&error.to_string()))?;
    }
    Ok(())
}

fn join(base: &str, path: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), path.trim_start_matches('/'))
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn bundled() -> CatalogIndex {
    serde_json::from_str(BUNDLED).unwrap_or(CatalogIndex { version: 1, notebooks: Vec::new() })
}

async fn read_capped(mut response: reqwest::Response, cap: usize) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        body.extend_from_slice(&chunk);
        if body.len() > cap {
            return Err(Error::invalid("The catalog response is too large."));
        }
    }
    Ok(body)
}

pub struct Catalog {
    http: reqwest::Client,
    base_url: RwLock<String>,
    cache_dir: PathBuf,
    state: Mutex<Loaded>,
}

impl Catalog {
    pub fn new(http: reqwest::Client, base_url: &str, cache_dir: PathBuf) -> Self {
        let catalog = Self {
            http,
            base_url: RwLock::new(base_url.to_owned()),
            cache_dir,
            state: Mutex::new(Loaded { index: bundled(), origin: CatalogOrigin::Bundled, fetched_at: None, error: None }),
        };
        if let Some((index, meta)) = catalog.read_cache() {
            *catalog.lock() = Loaded { index, origin: CatalogOrigin::Cache, fetched_at: meta.fetched_at, error: None };
        }
        catalog
    }

    fn lock(&self) -> MutexGuard<'_, Loaded> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn base_url(&self) -> String {
        self.base_url.read().map(|url| url.clone()).unwrap_or_else(|_| DEFAULT_CATALOG_URL.to_owned())
    }

    /// Point at another catalog (Settings); the next refresh loads it.
    pub fn set_base_url(&self, url: &str) {
        if let Ok(mut current) = self.base_url.write() {
            *current = url.to_owned();
        }
    }

    fn index_path(&self) -> PathBuf {
        self.cache_dir.join("index.json")
    }

    fn meta_path(&self) -> PathBuf {
        self.cache_dir.join("meta.json")
    }

    fn source_path(&self, sha: &str) -> PathBuf {
        self.cache_dir.join("sources").join(format!("{sha}.py"))
    }

    /// The cached index, if it belongs to the current catalog URL and is valid.
    fn read_cache(&self) -> Option<(CatalogIndex, CacheMeta)> {
        let meta: CacheMeta = serde_json::from_str(&std::fs::read_to_string(self.meta_path()).ok()?).ok()?;
        if meta.url != self.base_url() {
            return None;
        }
        let index: CatalogIndex = serde_json::from_str(&std::fs::read_to_string(self.index_path()).ok()?).ok()?;
        validate_index(&index).ok()?;
        Some((index, meta))
    }

    pub fn status(&self) -> CatalogStatus {
        let state = self.lock();
        CatalogStatus {
            origin: state.origin,
            url: self.base_url(),
            fetched_at: state.fetched_at.clone(),
            error: state.error.clone(),
            count: state.index.notebooks.len(),
        }
    }

    pub fn entries(&self) -> Vec<CatalogEntry> {
        self.lock().index.notebooks.clone()
    }

    pub fn entry(&self, slug: &str) -> Option<CatalogEntry> {
        self.lock().index.notebooks.iter().find(|entry| entry.slug == slug).cloned()
    }

    /// Fetch (or revalidate) the index. Never fails: problems are reported
    /// in the status and the previous copy stays in use.
    pub async fn refresh(&self) -> CatalogStatus {
        match self.fetch_index().await {
            Ok(loaded) => *self.lock() = loaded,
            Err(error) => {
                tracing::info!("Catalog refresh failed: {error}");
                self.lock().error = Some(error.to_string());
            }
        }
        self.status()
    }

    async fn fetch_index(&self) -> Result<Loaded> {
        let base = self.base_url();
        let cached = self.read_cache();
        let mut request = self.http.get(join(&base, "index.json")).timeout(Duration::from_secs(20));
        if let Some(etag) = cached.as_ref().and_then(|(_, meta)| meta.etag.clone()) {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        let response = request.send().await?;
        let fetched_at = now();

        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            let (index, mut meta) =
                cached.ok_or_else(|| Error::Network("The catalog answered 304 without a cached copy.".into()))?;
            meta.fetched_at = Some(fetched_at.clone());
            self.write_meta(&meta);
            return Ok(Loaded { index, origin: CatalogOrigin::Remote, fetched_at: Some(fetched_at), error: None });
        }
        if !response.status().is_success() {
            return Err(Error::Network(format!("The catalog answered {}.", response.status().as_u16())));
        }
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = read_capped(response, MAX_INDEX_BYTES).await?;
        let mut index: CatalogIndex = serde_json::from_slice(&body)
            .map_err(|error| Error::invalid(format!("The catalog index is malformed: {error}")))?;
        // Remote entries never carry inline sources.
        for entry in &mut index.notebooks {
            entry.source_text = None;
        }
        validate_index(&index)?;

        if let Err(error) = write_private(&self.index_path(), &body) {
            tracing::warn!("Could not cache the catalog: {error}");
        }
        self.write_meta(&CacheMeta { url: base, etag, fetched_at: Some(fetched_at.clone()) });
        Ok(Loaded { index, origin: CatalogOrigin::Remote, fetched_at: Some(fetched_at), error: None })
    }

    fn write_meta(&self, meta: &CacheMeta) {
        if let Ok(bytes) = serde_json::to_vec_pretty(meta) {
            if let Err(error) = write_private(&self.meta_path(), &bytes) {
                tracing::warn!("Could not cache the catalog metadata: {error}");
            }
        }
    }

    /// A notebook's source, verified against the catalog's SHA-256.
    pub async fn source(&self, slug: &str) -> Result<String> {
        let entry = self.entry(slug).ok_or_else(|| Error::not_found("Notebook not found."))?;
        if let Some(text) = entry.source_text.as_deref() {
            return verified(text.as_bytes(), &entry.sha256);
        }
        let cached = self.source_path(&entry.sha256);
        if let Ok(bytes) = std::fs::read(&cached) {
            if let Ok(text) = verified(&bytes, &entry.sha256) {
                return Ok(text);
            }
        }
        let fetched = self.fetch_source(&entry).await;
        match fetched {
            Ok(text) => {
                if let Err(error) = write_private(&cached, text.as_bytes()) {
                    tracing::warn!("Could not cache notebook source: {error}");
                }
                Ok(text)
            }
            // Offline, but the app ships this exact version.
            Err(error) => bundled()
                .notebooks
                .into_iter()
                .find(|bundled| bundled.slug == slug && bundled.sha256 == entry.sha256)
                .and_then(|bundled| bundled.source_text)
                .ok_or(error),
        }
    }

    async fn fetch_source(&self, entry: &CatalogEntry) -> Result<String> {
        let response = self
            .http
            .get(join(&self.base_url(), &entry.source))
            .timeout(Duration::from_secs(30))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(Error::Network(format!(
                "Could not download {} ({}).",
                entry.slug,
                response.status().as_u16()
            )));
        }
        let body = read_capped(response, MAX_SOURCE_BYTES).await?;
        verified(&body, &entry.sha256)
    }
}

fn verified(bytes: &[u8], sha: &str) -> Result<String> {
    if sha256_hex(bytes) != sha {
        return Err(Error::invalid(
            "The notebook failed its integrity check (SHA-256 mismatch) and was not run.",
        ));
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| Error::invalid("The notebook source is not UTF-8."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_snapshot_is_valid_and_verifiable() {
        let index = bundled();
        assert!(index.notebooks.len() >= 2);
        validate_index(&index).unwrap();
        for entry in &index.notebooks {
            let text = entry.source_text.as_deref().unwrap();
            assert_eq!(sha256_hex(text.as_bytes()), entry.sha256, "{}", entry.slug);
        }
        assert!(index.notebooks.iter().any(|entry| entry.slug == "print-notebook"));
    }

    #[test]
    fn untrusted_entries_are_rejected() {
        let good = CatalogEntry {
            slug: "ok-one".into(),
            title: "OK".into(),
            description: String::new(),
            tags: vec![],
            author: None,
            params: vec![],
            source: "notebooks/ok-one/notebook.py".into(),
            sha256: "a".repeat(64),
            source_text: None,
        };
        let index = |entries: Vec<CatalogEntry>| CatalogIndex { version: 1, notebooks: entries };
        assert!(validate_index(&index(vec![good.clone()])).is_ok());
        let variants = [
            CatalogEntry { slug: "Bad".into(), ..good.clone() },
            CatalogEntry { source: "../../etc/passwd.py".into(), ..good.clone() },
            CatalogEntry { source: "/abs.py".into(), ..good.clone() },
            CatalogEntry { source: "https://evil/x.py".into(), ..good.clone() },
            CatalogEntry { sha256: "A".repeat(64), ..good.clone() },
            CatalogEntry { title: " ".into(), ..good.clone() },
        ];
        for bad in variants {
            assert!(validate_index(&index(vec![bad.clone()])).is_err(), "{bad:?}");
        }
        assert!(validate_index(&index(vec![good.clone(), good.clone()])).is_err());
        assert!(validate_index(&CatalogIndex { version: 2, notebooks: vec![] }).is_err());
    }

    #[test]
    fn integrity_check() {
        let sha = sha256_hex(b"print(1)\n");
        assert_eq!(verified(b"print(1)\n", &sha).unwrap(), "print(1)\n");
        assert!(verified(b"print(2)\n", &sha).is_err());
        assert_eq!(join("https://x/y/", "/index.json"), "https://x/y/index.json");
    }
}
