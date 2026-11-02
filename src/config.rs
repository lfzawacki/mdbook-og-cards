use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Configuration for the og-cards preprocessor, parsed from the
/// `[preprocessor.og-cards]` table in `book.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Directory (relative to the book root, i.e. the directory containing
    /// `book.toml`) where fetched metadata is cached between builds.
    pub cache_dir: String,
    /// How long successfully-fetched metadata stays valid, in seconds.
    pub cache_ttl: u64,
    /// How long a failed fetch is remembered (to avoid hammering a broken
    /// URL on every build), in seconds.
    pub failure_cache_ttl: u64,
    /// Per-request timeout, in seconds.
    pub timeout: u64,
    /// Maximum number of in-flight HTTP requests at once.
    pub max_concurrent_requests: usize,
    /// User-Agent header sent with outgoing requests.
    pub user_agent: String,
    /// Whether generated cards open in a new tab (`target="_blank"`).
    pub open_in_new_tab: bool,
    /// Domains that should never be converted into cards, even if they
    /// match a trigger (link title `(preview)` or `{{#og url}}`).
    pub exclude_domains: HashSet<String>,
    /// Maximum number of redirects to follow before giving up.
    pub max_redirects: u8,
    /// Maximum response body size, in bytes, before aborting the fetch.
    pub max_response_bytes: u64,
    /// Test-only escape hatch to allow connecting to private/loopback IPs
    /// (e.g. a local mock HTTP server). Never set from `book.toml`; real
    /// book builds always go through full SSRF validation.
    #[serde(skip)]
    pub allow_private_ips_for_tests: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            cache_dir: ".cache/mdbook-og-cards".to_string(),
            cache_ttl: 7 * 24 * 60 * 60,
            failure_cache_ttl: 60 * 60,
            timeout: 10,
            max_concurrent_requests: 5,
            user_agent: format!("mdbook-og-cards/{}", env!("CARGO_PKG_VERSION")),
            open_in_new_tab: true,
            exclude_domains: HashSet::new(),
            max_redirects: 5,
            max_response_bytes: 2 * 1024 * 1024,
            allow_private_ips_for_tests: false,
        }
    }
}

impl Config {
    /// Build a `Config` from the raw TOML-derived JSON value mdbook gives
    /// preprocessors via `PreprocessorContext::config`.
    pub fn from_value(value: Option<&serde_json::Value>) -> Self {
        match value {
            Some(v) => {
                // Deserialize leniently: unknown/missing keys fall back to
                // defaults instead of failing the whole build.
                let mut cfg = Config::default();
                if let Some(obj) = v.as_object() {
                    if let Some(s) = obj.get("cache-dir").and_then(|x| x.as_str()) {
                        cfg.cache_dir = s.to_string();
                    }
                    if let Some(n) = obj.get("cache-ttl").and_then(|x| x.as_u64()) {
                        cfg.cache_ttl = n;
                    }
                    if let Some(n) = obj.get("failure-cache-ttl").and_then(|x| x.as_u64()) {
                        cfg.failure_cache_ttl = n;
                    }
                    if let Some(n) = obj.get("timeout").and_then(|x| x.as_u64()) {
                        cfg.timeout = n;
                    }
                    if let Some(n) = obj
                        .get("max-concurrent-requests")
                        .and_then(|x| x.as_u64())
                    {
                        cfg.max_concurrent_requests = n as usize;
                    }
                    if let Some(s) = obj.get("user-agent").and_then(|x| x.as_str()) {
                        cfg.user_agent = s.to_string();
                    }
                    if let Some(b) = obj.get("open-in-new-tab").and_then(|x| x.as_bool()) {
                        cfg.open_in_new_tab = b;
                    }
                    if let Some(arr) = obj.get("exclude-domains").and_then(|x| x.as_array()) {
                        cfg.exclude_domains = arr
                            .iter()
                            .filter_map(|x| x.as_str())
                            .map(|s| s.to_lowercase())
                            .collect();
                    }
                    if let Some(n) = obj.get("max-redirects").and_then(|x| x.as_u64()) {
                        cfg.max_redirects = n as u8;
                    }
                    if let Some(n) = obj.get("max-response-bytes").and_then(|x| x.as_u64()) {
                        cfg.max_response_bytes = n;
                    }
                    // Undocumented escape hatch used only by this crate's own
                    // test suite to point at local mock HTTP servers on
                    // loopback addresses. Not intended for use in a real
                    // book.toml; SSRF protection should stay enabled for any
                    // book that fetches links from untrusted content.
                    if let Some(b) = obj
                        .get("allow-private-ips-for-tests")
                        .and_then(|x| x.as_bool())
                    {
                        cfg.allow_private_ips_for_tests = b;
                    }
                }
                cfg
            }
            None => Config::default(),
        }
    }

    pub fn is_domain_excluded(&self, host: &str) -> bool {
        let host = host.to_lowercase();
        self.exclude_domains.iter().any(|d| {
            let d = d.trim_start_matches("www.");
            let h = host.trim_start_matches("www.");
            h == d || h.ends_with(&format!(".{d}"))
        })
    }
}
