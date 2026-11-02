use crate::extractor::OgMetadata;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FetchOutcome {
    Success(OgMetadata),
    Failure { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    fetched_at: u64,
    outcome: FetchOutcome,
}

pub struct Cache {
    dir: PathBuf,
    success_ttl: u64,
    failure_ttl: u64,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn cache_key(canonical_url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical_url.as_bytes());
    format!("{:x}", hasher.finalize())
}

impl Cache {
    pub fn open(dir: impl Into<PathBuf>, success_ttl: u64, failure_ttl: u64) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)
            .with_context(|| format!("creating cache directory {}", dir.display()))?;
        Ok(Cache {
            dir,
            success_ttl,
            failure_ttl,
        })
    }

    fn path_for(&self, url: &str) -> PathBuf {
        self.dir.join(format!("{}.json", cache_key(url)))
    }

    /// Returns a cached outcome if present and not expired.
    pub fn get(&self, url: &str) -> Option<FetchOutcome> {
        let path = self.path_for(url);
        let content = fs::read_to_string(&path).ok()?;
        let entry: CacheEntry = serde_json::from_str(&content).ok()?;
        let ttl = match &entry.outcome {
            FetchOutcome::Success(_) => self.success_ttl,
            FetchOutcome::Failure { .. } => self.failure_ttl,
        };
        if ttl == 0 || now().saturating_sub(entry.fetched_at) > ttl {
            return None;
        }
        Some(entry.outcome)
    }

    pub fn put(&self, url: &str, outcome: FetchOutcome) -> Result<()> {
        let entry = CacheEntry {
            fetched_at: now(),
            outcome,
        };
        let path = self.path_for(url);
        let json = serde_json::to_string_pretty(&entry)?;
        fs::write(&path, json).with_context(|| format!("writing cache file {}", path.display()))?;
        Ok(())
    }

    /// Remove all cached entries. Used by the `--clear-cache` CLI flag.
    pub fn clear(dir: &Path) -> Result<()> {
        if dir.exists() {
            fs::remove_dir_all(dir)
                .with_context(|| format!("removing cache directory {}", dir.display()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_retrieves_success_entry() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path(), 3600, 60).unwrap();
        let meta = OgMetadata {
            title: Some("Title".into()),
            description: None,
            image: None,
            site_name: None,
            url: "https://example.com".into(),
        };
        cache
            .put("https://example.com", FetchOutcome::Success(meta.clone()))
            .unwrap();
        match cache.get("https://example.com") {
            Some(FetchOutcome::Success(m)) => assert_eq!(m, meta),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn expires_failure_entries_faster_than_success() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path(), 3600, 0).unwrap();
        cache
            .put(
                "https://example.com/broken",
                FetchOutcome::Failure {
                    reason: "404".into(),
                },
            )
            .unwrap();
        // failure_ttl = 0, so this should already be considered expired.
        assert!(cache.get("https://example.com/broken").is_none());
    }

    #[test]
    fn missing_entry_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path(), 3600, 60).unwrap();
        assert!(cache.get("https://example.com/missing").is_none());
    }
}
