use anyhow::Result;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use url::Url;

/// Open Graph (plus fallback) metadata extracted from a fetched page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OgMetadata {
    pub title: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
    pub site_name: Option<String>,
    pub url: String,
}

fn meta_content(doc: &Html, selector_str: &str) -> Option<String> {
    let selector = Selector::parse(selector_str).ok()?;
    doc.select(&selector)
        .next()
        .and_then(|el| el.value().attr("content"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Extract Open Graph metadata from an HTML document, following the
/// priority order: og:* -> twitter:* -> plain HTML fallbacks.
pub fn extract(html: &str, page_url: &str) -> Result<OgMetadata> {
    let doc = Html::parse_document(html);

    let title = meta_content(&doc, r#"meta[property="og:title"]"#)
        .or_else(|| meta_content(&doc, r#"meta[name="twitter:title"]"#))
        .or_else(|| {
            Selector::parse("title")
                .ok()
                .and_then(|sel| doc.select(&sel).next())
                .map(|el| el.text().collect::<String>().trim().to_string())
                .filter(|s| !s.is_empty())
        });

    let description = meta_content(&doc, r#"meta[property="og:description"]"#)
        .or_else(|| meta_content(&doc, r#"meta[name="twitter:description"]"#))
        .or_else(|| meta_content(&doc, r#"meta[name="description"]"#));

    let raw_image = meta_content(&doc, r#"meta[property="og:image"]"#)
        .or_else(|| meta_content(&doc, r#"meta[name="twitter:image"]"#));

    let site_name = meta_content(&doc, r#"meta[property="og:site_name"]"#).or_else(|| {
        Url::parse(page_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
    });

    let canonical_url =
        meta_content(&doc, r#"meta[property="og:url"]"#).unwrap_or_else(|| page_url.to_string());

    // Resolve relative image URLs against the (possibly redirected) page URL.
    let image = raw_image.and_then(|img| resolve_url(page_url, &img));

    Ok(OgMetadata {
        title,
        description,
        image,
        site_name,
        url: canonical_url,
    })
}

fn resolve_url(base: &str, maybe_relative: &str) -> Option<String> {
    if maybe_relative.starts_with("http://") || maybe_relative.starts_with("https://") {
        return Some(maybe_relative.to_string());
    }
    // Protocol-relative URLs, e.g. //cdn.example.com/img.png
    if let Some(stripped) = maybe_relative.strip_prefix("//") {
        let scheme = Url::parse(base).ok()?.scheme().to_string();
        return Some(format!("{scheme}://{stripped}"));
    }
    let base_url = Url::parse(base).ok()?;
    base_url.join(maybe_relative).ok().map(|u| u.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_full_og_metadata() {
        let html = r#"<html><head>
            <meta property="og:title" content="mdBook">
            <meta property="og:description" content="Create books from Markdown">
            <meta property="og:image" content="https://example.com/img.png">
            <meta property="og:site_name" content="GitHub">
        </head></html>"#;
        let data = extract(html, "https://github.com/rust-lang/mdBook").unwrap();
        assert_eq!(data.title.as_deref(), Some("mdBook"));
        assert_eq!(
            data.description.as_deref(),
            Some("Create books from Markdown")
        );
        assert_eq!(data.image.as_deref(), Some("https://example.com/img.png"));
        assert_eq!(data.site_name.as_deref(), Some("GitHub"));
    }

    #[test]
    fn falls_back_to_html_title_and_meta_description() {
        let html = r#"<html><head>
            <title>Fallback Title</title>
            <meta name="description" content="Fallback description">
        </head></html>"#;
        let data = extract(html, "https://example.com/page").unwrap();
        assert_eq!(data.title.as_deref(), Some("Fallback Title"));
        assert_eq!(data.description.as_deref(), Some("Fallback description"));
        assert_eq!(data.image, None);
        assert_eq!(data.site_name.as_deref(), Some("example.com"));
    }

    #[test]
    fn resolves_relative_image_url() {
        let html = r#"<html><head>
            <meta property="og:title" content="Page">
            <meta property="og:image" content="/static/img.png">
        </head></html>"#;
        let data = extract(html, "https://example.com/some/page").unwrap();
        assert_eq!(data.image.as_deref(), Some("https://example.com/static/img.png"));
    }

    #[test]
    fn handles_missing_metadata_gracefully() {
        let html = "<html><head></head><body>no meta here</body></html>";
        let data = extract(html, "https://example.com/").unwrap();
        assert_eq!(data.title, None);
        assert_eq!(data.description, None);
        assert_eq!(data.image, None);
    }
}
