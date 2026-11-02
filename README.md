# mdbook-og-cards

An [mdBook](https://rust-lang.github.io/mdBook/) preprocessor that converts
explicitly-marked links into rich Open Graph preview cards at **build
time**. Generated books need no JavaScript, no runtime API calls, and no
client-side dependencies — all metadata is fetched once during `mdbook
build` and baked into static HTML.

## Why "explicit" marking?

Markdown files routinely contain many external links (citations, footnotes,
references) that should stay as plain links. Converting *every* link into a
card would be slow (one HTTP request per link, every build) and would wreck
long reference lists. Instead, `mdbook-og-cards` only converts links you
explicitly mark, using two mechanisms:

### 1. The `(preview)` link title

Any CommonMark link whose title is exactly `preview` is converted:

```markdown
Check out [mdBook](https://github.com/rust-lang/mdBook (preview)).
```

This is valid, standard Markdown — parenthesized titles are part of the
CommonMark spec — so it degrades gracefully (as a normal link with a
tooltip) if the preprocessor isn't installed. It's also exactly the
convention already produced by this book's note-sync tooling for
standalone reference links.

### 2. The `{{#og URL}}` shortcode

For cases where you want a card without an accompanying link/sentence,
drop a shortcode on its own line (mirrors the existing `{{#embed}}`
convention):

```markdown
{{#og https://github.com/rust-lang/mdBook}}
```

Everything else — bare citation URLs, normal `[text](url "title")` links,
autolinks, links/images inside code blocks — is left completely untouched.

## Installation

```bash
cargo install --path mdbook-og-cards --locked
```

(or, once published, `cargo install mdbook-og-cards`)

## Configuration

Add to `book.toml`:

```toml
[preprocessor.og-cards]
command = "mdbook-og-cards"
renderers = ["html"]
```

And include the stylesheet (referencing it directly keeps a single source
of truth; copying it into your book's `theme/` directory also works):

```toml
[output.html]
additional-css = ["mdbook-og-cards/styles/og-cards.css"]
```

### Options

All keys are optional; defaults shown below.

```toml
[preprocessor.og-cards]
command = "mdbook-og-cards"
renderers = ["html"]
cache-dir = ".cache/mdbook-og-cards"   # relative to book.toml's directory
cache-ttl = 604800                      # 7 days, in seconds
failure-cache-ttl = 3600                 # 1 hour, in seconds
timeout = 10                             # per-request timeout, in seconds
max-concurrent-requests = 5
user-agent = "mdbook-og-cards/<version>"
open-in-new-tab = true
exclude-domains = []                     # e.g. ["localhost", "example.org"]
max-redirects = 5
max-response-bytes = 2097152             # 2 MiB
```

## Caching

Fetched metadata is cached to disk as JSON, keyed by the SHA-256 hash of
the canonical URL, under `cache-dir`. Successful fetches are cached for
`cache-ttl` seconds; failures are cached for a much shorter
`failure-cache-ttl` so a broken link doesn't get hammered on every build
but also doesn't poison the build forever.

Because `cache-dir` defaults to living next to `book.toml` (not inside the
generated `book/` output directory), it survives `mdbook clean` and can be
restored across CI runs with `actions/cache` (see this repo's
`.github/workflows/deploy.yml` for an example), so unchanged URLs are never
refetched on every push.

To clear the cache:

```bash
mdbook-og-cards --clear-cache .cache/mdbook-og-cards
```

## Security

Because this preprocessor fetches URLs found in Markdown content, it
implements SSRF protections:

- Only `http`/`https` schemes are allowed.
- Every hostname is DNS-resolved and validated — loopback, private
  (RFC1918), link-local, carrier-grade NAT, multicast, unspecified, and
  IPv6 unique-local/link-local addresses are all rejected.
- Redirects are followed manually (not via reqwest's automatic follower)
  so **every hop** is re-validated before connecting — a redirect to an
  internal address is blocked just like a direct request would be.
- Response bodies are capped (`max-response-bytes`) to avoid downloading
  huge files.
- All extracted metadata (title, description, site name) is HTML-escaped
  before being inserted into generated pages — remote sites' metadata is
  never trusted as safe HTML.

This does **not** make it safe to point at arbitrary/untrusted
third-party-controlled URLs without review — it only prevents the most
common SSRF footguns (hitting your own internal network / cloud metadata
endpoints).

## Failure behavior

If a URL can't be fetched (timeout, 404, DNS failure, excluded domain,
blocked by SSRF checks, etc.), the original link is preserved verbatim as
a plain `<a href="...">` — a failed fetch never breaks the build.

## CSS customization

`styles/og-cards.css` styles the `.og-card` flex container, the
`.og-card-image` thumbnail, and the `.og-card-body` / `.og-card-title` /
`.og-card-description` / `.og-card-source` text column. It switches from
an image-left layout to image-top at `480px` and respects mdBook's default
theme variables (works with light and dark themes).

The generated markup uses only inline/phrasing elements (`a`, `img`,
`span`, `strong`, `small`) because cards are often spliced into a
paragraph; block-level tags would make the HTML5 parser close the
paragraph and mangle the DOM. Override or replace the stylesheet freely —
the generated markup intentionally has no inline styles.

## Development

```bash
cargo test
```

Tests use [`wiremock`](https://docs.rs/wiremock) to mock HTTP responses —
no network access or external services are required.
