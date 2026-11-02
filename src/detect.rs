use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::ops::Range;

/// A spot in the original Markdown source that should be replaced with an
/// OG card (or its fallback), along with the URL to fetch metadata for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub range: Range<usize>,
    pub url: String,
    pub kind: TriggerKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerKind {
    /// `[text](url (preview))` - CommonMark parenthesized link title.
    PreviewTitle,
    /// `{{#og url}}` shortcode on its own line.
    Shortcode,
}

const PREVIEW_TITLE: &str = "preview";

fn is_http(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Scan Markdown source for link-title `(preview)` triggers using
/// pulldown-cmark's offset-tracking parser, so we automatically skip code
/// blocks/spans, images, and other non-link contexts.
fn find_preview_title_links(content: &str) -> Vec<Candidate> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(content, options).into_offset_iter();
    let mut candidates = Vec::new();
    let mut open_link: Option<(Range<usize>, String)> = None;

    for (event, range) in parser {
        match event {
            Event::Start(Tag::Link {
                dest_url, title, ..
            }) => {
                if title.trim().eq_ignore_ascii_case(PREVIEW_TITLE) && is_http(&dest_url) {
                    open_link = Some((range, dest_url.to_string()));
                } else {
                    open_link = None;
                }
            }
            Event::End(TagEnd::Link) => {
                if let Some((start_range, url)) = open_link.take() {
                    // Use the link start event's range; pulldown-cmark gives
                    // the full span (including closing paren) on the Start
                    // event range for inline links in practice, but to be
                    // safe we extend to the End event's range end too.
                    let end = range.end.max(start_range.end);
                    candidates.push(Candidate {
                        range: start_range.start..end,
                        url,
                        kind: TriggerKind::PreviewTitle,
                    });
                }
            }
            _ => {}
        }
    }

    candidates
}

/// Scan line-by-line for `{{#og URL}}` shortcodes, skipping fenced code
/// blocks. This mirrors the existing `{{#embed URL}}` convention already
/// used in this book.
fn find_shortcodes(content: &str) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    let mut in_fence = false;
    let mut offset = 0usize;

    for line in content.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            offset += line.len();
            continue;
        }
        if !in_fence {
            if let Some(url) = parse_og_shortcode(trimmed) {
                let start = offset + (line.len() - line.trim_start().len());
                let line_trimmed_end = line.trim_end_matches(['\n', '\r']);
                let end = offset + line_trimmed_end.len();
                candidates.push(Candidate {
                    range: start..end,
                    url,
                    kind: TriggerKind::Shortcode,
                });
            }
        }
        offset += line.len();
    }

    candidates
}

fn parse_og_shortcode(trimmed_line: &str) -> Option<String> {
    let rest = trimmed_line.strip_prefix("{{#og")?;
    // Avoid accidentally matching e.g. {{#oggy}} or other future shortcodes.
    let rest = rest.strip_prefix(char::is_whitespace)?;
    let rest = rest.trim_end();
    let rest = rest.strip_suffix("}}")?;
    let url = rest.trim();
    if is_http(url) && !url.contains(char::is_whitespace) {
        Some(url.to_string())
    } else {
        None
    }
}

/// Find all conversion candidates in a chapter's content, sorted by
/// descending start offset so callers can safely splice replacements into
/// the source without invalidating earlier ranges.
pub fn find_candidates(content: &str) -> Vec<Candidate> {
    let mut candidates = find_preview_title_links(content);
    candidates.extend(find_shortcodes(content));
    candidates.sort_by_key(|c| std::cmp::Reverse(c.range.start));
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_preview_title_link() {
        let md = "Check out [mdBook](https://example.com/mdbook (preview)) today.";
        let cands = find_candidates(md);
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].url, "https://example.com/mdbook");
        assert_eq!(cands[0].kind, TriggerKind::PreviewTitle);
    }

    #[test]
    fn ignores_normal_title_link() {
        let md = "[Wikipedia](https://example.com \"Some Title\")";
        let cands = find_candidates(md);
        assert!(cands.is_empty());
    }

    #[test]
    fn ignores_bare_url() {
        let md = "https://example.com/some/citation\n";
        let cands = find_candidates(md);
        assert!(cands.is_empty());
    }

    #[test]
    fn ignores_link_inside_code_block() {
        let md = "```\n[x](https://example.com (preview))\n```\n";
        let cands = find_candidates(md);
        assert!(cands.is_empty());
    }

    #[test]
    fn detects_shortcode() {
        let md = "Some text\n\n{{#og https://example.com/page}}\n\nMore text";
        let cands = find_candidates(md);
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].url, "https://example.com/page");
        assert_eq!(cands[0].kind, TriggerKind::Shortcode);
    }

    #[test]
    fn ignores_shortcode_in_fence() {
        let md = "```\n{{#og https://example.com/page}}\n```\n";
        let cands = find_candidates(md);
        assert!(cands.is_empty());
    }

    #[test]
    fn ignores_non_http_preview_title() {
        let md = "[local](./file.md (preview))";
        let cands = find_candidates(md);
        assert!(cands.is_empty());
    }
}
