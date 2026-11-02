use crate::cache::{Cache, FetchOutcome};
use crate::config::Config;
use crate::detect::{find_candidates, Candidate};
use crate::extractor;
use crate::fetch::fetch_html;
use crate::renderer::{render_card, render_fallback};
use mdbook::book::{Book, BookItem};
use mdbook::errors::Error;
use mdbook::preprocess::{Preprocessor, PreprocessorContext};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Semaphore;

pub struct OgCardsPreprocessor;

impl OgCardsPreprocessor {
    pub fn new() -> Self {
        OgCardsPreprocessor
    }
}

impl Default for OgCardsPreprocessor {
    fn default() -> Self {
        Self::new()
    }
}

impl Preprocessor for OgCardsPreprocessor {
    fn name(&self) -> &str {
        "og-cards"
    }

    fn run(&self, ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        let raw_config = ctx
            .config
            .get_preprocessor(self.name())
            .map(|table| serde_json::to_value(table).unwrap_or(serde_json::Value::Null));
        let config = Config::from_value(raw_config.as_ref());

        // Resolve cache dir relative to the book root (directory containing
        // book.toml), not the chapter src dir or the output dir.
        let cache_dir = ctx.root.join(&config.cache_dir);
        let cache = Cache::open(&cache_dir, config.cache_ttl, config.failure_cache_ttl)
            .map_err(|e| Error::msg(format!("opening og-cards cache: {e}")))?;

        // Collect every candidate URL across all chapters first, so we can
        // fetch them concurrently (bounded) in a single async batch, then do
        // a second synchronous pass to splice the rendered HTML back in.
        let mut all_candidates: Vec<(Vec<usize>, Vec<Candidate>)> = Vec::new();
        let mut urls_to_fetch: Vec<String> = Vec::new();

        let mut chapter_paths: Vec<Vec<usize>> = Vec::new();
        collect_chapter_paths(&book, &mut Vec::new(), &mut chapter_paths);

        for path in &chapter_paths {
            if let Some(BookItem::Chapter(chapter)) = get_item(&book, path) {
                let candidates = find_candidates(&chapter.content);
                for c in &candidates {
                    if let Ok(host) = url::Url::parse(&c.url)
                        .map(|u| u.host_str().unwrap_or_default().to_string())
                    {
                        if config.is_domain_excluded(&host) {
                            continue;
                        }
                    }
                    urls_to_fetch.push(c.url.clone());
                }
                all_candidates.push((path.clone(), candidates));
            }
        }

        urls_to_fetch.sort();
        urls_to_fetch.dedup();

        let outcomes = fetch_all(&config, &cache, urls_to_fetch)
            .map_err(|e| Error::msg(format!("fetching og-cards metadata: {e}")))?;

        for (path, candidates) in all_candidates {
            if candidates.is_empty() {
                continue;
            }
            if let Some(BookItem::Chapter(chapter)) = get_item_mut(&mut book, &path) {
                let mut content = chapter.content.clone();
                // candidates are sorted by descending start offset already.
                for c in &candidates {
                    let host_excluded = url::Url::parse(&c.url)
                        .ok()
                        .and_then(|u| u.host_str().map(|h| config.is_domain_excluded(h)))
                        .unwrap_or(false);

                    let replacement = if host_excluded {
                        None
                    } else {
                        match outcomes.get(&c.url) {
                            Some(FetchOutcome::Success(meta)) => {
                                Some(render_card(meta, config.open_in_new_tab))
                            }
                            Some(FetchOutcome::Failure { .. }) | None => {
                                Some(render_fallback(&c.url))
                            }
                        }
                    };

                    if let Some(html) = replacement {
                        if c.range.start <= content.len() && c.range.end <= content.len() {
                            content.replace_range(c.range.clone(), &html);
                        }
                    }
                }
                chapter.content = content;
            }
        }

        Ok(book)
    }

    fn supports_renderer(&self, renderer: &str) -> bool {
        renderer == "html"
    }
}

/// Fetch (or load from cache) metadata for every URL, bounded by
/// `max_concurrent_requests` in flight at once. Returns a map from URL to
/// outcome; URLs that fail are recorded as `FetchOutcome::Failure` so the
/// caller can render a fallback link.
fn fetch_all(
    config: &Config,
    cache: &Cache,
    urls: Vec<String>,
) -> anyhow::Result<HashMap<String, FetchOutcome>> {
    let mut results = HashMap::new();
    let mut pending = Vec::new();

    for url in urls {
        match cache.get(&url) {
            Some(outcome) => {
                results.insert(url, outcome);
            }
            None => pending.push(url),
        }
    }

    if pending.is_empty() {
        return Ok(results);
    }

    // If we're already being run from inside a tokio runtime (this only
    // happens in this crate's own test suite; real `mdbook build` always
    // invokes preprocessors from a plain synchronous context), reuse the
    // ambient runtime instead of trying to nest a new one, which tokio
    // forbids.
    let fetched = if let Ok(handle) = tokio::runtime::Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(fetch_pending(config, pending)))?
    } else {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(fetch_pending(config, pending))?
    };

    for (url, outcome) in fetched {
        if let Err(e) = cache.put(&url, outcome.clone()) {
            eprintln!("og-cards: warning: failed to write cache entry for {url}: {e}");
        }
        results.insert(url, outcome);
    }

    Ok(results)
}

async fn fetch_pending(
    config: &Config,
    urls: Vec<String>,
) -> anyhow::Result<Vec<(String, FetchOutcome)>> {
    let semaphore = Arc::new(Semaphore::new(config.max_concurrent_requests.max(1)));
    let mut handles = Vec::new();

    for url in urls {
        let sem = semaphore.clone();
        let config = config.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore closed");
            let outcome = match fetch_html(&config, &url).await {
                Ok((html, final_url)) => match extractor::extract(&html, &final_url) {
                    Ok(meta) => FetchOutcome::Success(meta),
                    Err(e) => FetchOutcome::Failure {
                        reason: e.to_string(),
                    },
                },
                Err(e) => FetchOutcome::Failure {
                    reason: e.to_string(),
                },
            };
            (url, outcome)
        }));
    }

    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        out.push(h.await?);
    }
    Ok(out)
}

/// Walk the book tree collecting index-paths to every chapter, so we can
/// later look them up individually for both a read pass and a write pass.
fn collect_chapter_paths(book: &Book, prefix: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    for (i, item) in book.sections.iter().enumerate() {
        prefix.push(i);
        if let BookItem::Chapter(chapter) = item {
            out.push(prefix.clone());
            collect_in_chapter(chapter, prefix, out);
        }
        prefix.pop();
    }
}

fn collect_in_chapter(
    chapter: &mdbook::book::Chapter,
    prefix: &mut Vec<usize>,
    out: &mut Vec<Vec<usize>>,
) {
    for (i, sub) in chapter.sub_items.iter().enumerate() {
        prefix.push(i);
        if let BookItem::Chapter(c) = sub {
            out.push(prefix.clone());
            collect_in_chapter(c, prefix, out);
        }
        prefix.pop();
    }
}

fn get_item<'a>(book: &'a Book, path: &[usize]) -> Option<&'a BookItem> {
    let (&first, rest) = path.split_first()?;
    let mut item = book.sections.get(first)?;
    let mut rest = rest;
    loop {
        if rest.is_empty() {
            return Some(item);
        }
        let (&next, tail) = rest.split_first()?;
        if let BookItem::Chapter(chapter) = item {
            item = chapter.sub_items.get(next)?;
            rest = tail;
        } else {
            return None;
        }
    }
}

fn get_item_mut<'a>(book: &'a mut Book, path: &[usize]) -> Option<&'a mut BookItem> {
    let (&first, rest) = path.split_first()?;
    let mut item = book.sections.get_mut(first)?;
    let mut rest = rest;
    loop {
        if rest.is_empty() {
            return Some(item);
        }
        let (&next, tail) = rest.split_first()?;
        if let BookItem::Chapter(chapter) = item {
            item = chapter.sub_items.get_mut(next)?;
            rest = tail;
        } else {
            return None;
        }
    }
}
