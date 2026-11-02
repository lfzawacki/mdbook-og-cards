use crate::extractor::OgMetadata;

/// Minimal HTML escaping sufficient for attribute and text contexts.
pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn hostname(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| url.to_string())
}

/// Render a full OG preview card as raw HTML, suitable for splicing
/// directly into Markdown source (passed through untouched by mdBook).
///
/// This card is frequently spliced into the middle of flowing prose, which
/// Markdown wraps in a `<p>`. The HTML5 parsing algorithm auto-closes an
/// open `<p>` as soon as it sees a block-level start tag (`div`, another
/// `p`, etc.), which would tear the anchor apart and trigger the "adoption
/// agency" algorithm to duplicate it around the trailing content. To stay
/// valid (and render correctly) when nested inside a `<p>`, every element
/// here is an inline-level/phrasing tag (`a`, `span`, `strong`, `small`,
/// `img`) styled with CSS `display` overrides instead of relying on
/// semantically block-level tags.
pub fn render_card(meta: &OgMetadata, open_in_new_tab: bool) -> String {
    let href = escape_html(&meta.url);
    let site_name = meta
        .site_name
        .clone()
        .unwrap_or_else(|| hostname(&meta.url));
    let target_attr = if open_in_new_tab {
        r#" target="_blank" rel="noopener noreferrer""#
    } else {
        ""
    };

    let image_html = meta
        .image
        .as_ref()
        .map(|img| format!(r#"<img class="og-card-image" src="{}" alt="" loading="lazy">"#, escape_html(img)))
        .unwrap_or_default();

    let title_html = meta
        .title
        .as_deref()
        .map(escape_html)
        .unwrap_or_else(|| hostname(&meta.url));

    let description_html = meta
        .description
        .as_ref()
        .map(|d| format!(r#"<span class="og-card-description">{}</span>"#, escape_html(d)))
        .unwrap_or_default();

    format!(
        concat!(
            r#"<a class="og-card" href="{href}"{target_attr}>"#,
            "{image_html}",
            r#"<span class="og-card-body">"#,
            r#"<strong class="og-card-title">{title_html}</strong>"#,
            "{description_html}",
            r#"<small class="og-card-source">{site_name}</small>"#,
            "</span>",
            "</a>"
        ),
        href = href,
        target_attr = target_attr,
        image_html = image_html,
        title_html = title_html,
        description_html = description_html,
        site_name = escape_html(&site_name),
    )
}

/// Fallback markup when metadata could not be fetched: preserve the
/// original link so the build never breaks and readers still get a
/// clickable URL.
pub fn render_fallback(url: &str) -> String {
    let escaped = escape_html(url);
    format!(r#"<a href="{escaped}">{escaped}</a>"#)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(title: Option<&str>, desc: Option<&str>, image: Option<&str>) -> OgMetadata {
        OgMetadata {
            title: title.map(String::from),
            description: desc.map(String::from),
            image: image.map(String::from),
            site_name: None,
            url: "https://example.com/page".to_string(),
        }
    }

    #[test]
    fn renders_full_card() {
        let m = meta(Some("Title"), Some("Desc"), Some("https://example.com/img.png"));
        let html = render_card(&m, true);
        assert!(html.contains("og-card"));
        assert!(html.contains(r#"<strong class="og-card-title">Title</strong>"#));
        assert!(html.contains(r#"<span class="og-card-description">Desc</span>"#));
        assert!(html.contains("img.png"));
        assert!(html.contains("target=\"_blank\""));
        assert!(html.contains("rel=\"noopener noreferrer\""));
    }

    #[test]
    fn only_uses_inline_safe_tags() {
        // The card is frequently spliced into the middle of a Markdown
        // paragraph (wrapped in a `<p>` by the renderer). Any block-level
        // tag here (div, p, ul, etc.) would cause the HTML5 parser to
        // auto-close that `<p>` and mangle the DOM. Only inline/phrasing
        // tags are allowed.
        let m = meta(Some("Title"), Some("Desc"), Some("https://example.com/img.png"));
        let html = render_card(&m, true);
        for forbidden in ["<div", "<p>", "<p ", "<ul", "<table"] {
            assert!(
                !html.contains(forbidden),
                "card markup must not contain block-level tag {forbidden}, got: {html}"
            );
        }
    }

    #[test]
    fn omits_missing_image_and_description() {
        let m = meta(Some("Title"), None, None);
        let html = render_card(&m, false);
        assert!(!html.contains("<img"));
        assert!(!html.contains("og-card-description"));
        assert!(!html.contains("target=\"_blank\""));
    }

    #[test]
    fn escapes_html_in_metadata() {
        let m = meta(Some("<script>alert(1)</script>"), None, None);
        let html = render_card(&m, true);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn fallback_preserves_original_link() {
        let html = render_fallback("https://example.com/broken?a=1&b=2");
        assert!(html.contains("https://example.com/broken?a=1&amp;b=2"));
    }
}
