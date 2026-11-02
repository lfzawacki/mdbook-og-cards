//! End-to-end test of the `Preprocessor::run` implementation, exercising
//! the full detect -> fetch -> cache -> render pipeline against a real
//! `Book`/`PreprocessorContext` pair, using the same JSON wire protocol
//! mdBook itself uses to talk to preprocessor subprocesses.

use mdbook::book::{Book, Chapter};
use mdbook::preprocess::{CmdPreprocessor, Preprocessor};
use mdbook_og_cards::OgCardsPreprocessor;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Build a (PreprocessorContext, Book) pair the same way mdbook would when
/// invoking this preprocessor as a subprocess, then run it.
fn run_preprocessor(root: &std::path::Path, config_toml_table: serde_json::Value, book: Book) -> Book {
    let mut preprocessor_cfg = serde_json::Map::new();
    preprocessor_cfg.insert("og-cards".to_string(), config_toml_table);
    let mut preprocessor_table = serde_json::Map::new();
    preprocessor_table.insert("preprocessor".to_string(), serde_json::Value::Object(preprocessor_cfg));

    // mdbook's `Config` deserializer treats the whole incoming table as
    // flat: it pulls out `book`/`build`/`rust` and stashes everything else
    // (including `preprocessor.*`) in its internal `rest` field. So the
    // wire-format "config" value is just that flat table, not a nested
    // `{ book, build, rust, rest }` struct.
    let mut config_table = preprocessor_table;
    config_table.insert("book".to_string(), serde_json::json!({}));
    config_table.insert("build".to_string(), serde_json::json!({}));
    config_table.insert("rust".to_string(), serde_json::json!({}));

    let ctx_json = serde_json::json!({
        "root": root,
        "config": config_table,
        "renderer": "html",
        "mdbook_version": mdbook::MDBOOK_VERSION,
    });

    let input = serde_json::json!([ctx_json, book]);
    let bytes = serde_json::to_vec(&input).unwrap();

    let (ctx, book) = CmdPreprocessor::parse_input(bytes.as_slice()).unwrap();
    let pre = OgCardsPreprocessor::new();
    pre.run(&ctx, book).unwrap()
}

fn chapter_with(content: &str) -> Book {
    let mut book = Book::new();
    let ch = Chapter::new(
        "Test Chapter",
        content.to_string(),
        "test.md",
        Vec::new(),
    );
    book.push_item(ch);
    book
}

fn first_chapter_content(book: &Book) -> String {
    for item in book.iter() {
        if let mdbook::book::BookItem::Chapter(c) = item {
            return c.content.clone();
        }
    }
    panic!("no chapter found");
}

#[test]
fn converts_preview_titled_link_into_og_card() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/mdbook"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<html><head><meta property="og:title" content="mdBook"></head></html>"#,
            ))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let url = format!("{}/mdbook", server.uri());
        let md = format!("Check out [mdBook]({} (preview)).", url);
        let book = chapter_with(&md);

        let cfg = serde_json::json!({ "cache-dir": ".cache/test", "allow-private-ips-for-tests": true });
        let result = run_preprocessor(dir.path(), cfg, book);
        let content = first_chapter_content(&result);

        assert!(content.contains("og-card"), "expected og-card in: {content}");
        assert!(content.contains("mdBook"));
    });
}

#[test]
fn leaves_plain_links_and_bare_urls_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let md = "See https://example.com/citation and [a link](https://example.com \"title\").";
    let book = chapter_with(md);
    let cfg = serde_json::json!({ "cache-dir": ".cache/test2" });
    let result = run_preprocessor(dir.path(), cfg, book);
    let content = first_chapter_content(&result);
    assert_eq!(content, md);
}

#[test]
fn does_not_transform_link_inside_code_block() {
    let dir = tempfile::tempdir().unwrap();
    let md = "```\n[x](https://example.com/page (preview))\n```\n";
    let book = chapter_with(md);
    let cfg = serde_json::json!({ "cache-dir": ".cache/test3" });
    let result = run_preprocessor(dir.path(), cfg, book);
    let content = first_chapter_content(&result);
    assert_eq!(content, md);
}

#[test]
fn does_not_transform_markdown_image() {
    let dir = tempfile::tempdir().unwrap();
    let md = "![alt](https://example.com/image.png (preview))";
    let book = chapter_with(md);
    let cfg = serde_json::json!({ "cache-dir": ".cache/test4" });
    let result = run_preprocessor(dir.path(), cfg, book);
    let content = first_chapter_content(&result);
    // Images use Tag::Image, not Tag::Link, so our Link-only detector must
    // never touch this line.
    assert_eq!(content, md);
}

#[test]
fn invalid_url_falls_back_to_original_link() {
    let dir = tempfile::tempdir().unwrap();
    // Port 1 on localhost should fail to connect / be blocked by SSRF checks.
    let md = "[broken](http://127.0.0.1:1/nope (preview))";
    let book = chapter_with(md);
    let cfg = serde_json::json!({ "cache-dir": ".cache/test5" });
    let result = run_preprocessor(dir.path(), cfg, book);
    let content = first_chapter_content(&result);
    assert!(content.contains("http://127.0.0.1:1/nope"));
    assert!(!content.contains("og-card"));
}

#[test]
fn excluded_domain_falls_back_to_original_link() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = MockServer::start().await;
        // No mock registered for any path: if the preprocessor tried to
        // fetch this, wiremock would return a 404 "no matching mock" error,
        // which would also result in a fallback link. To make the "excluded
        // domain" assertion meaningful, we explicitly exclude the mock
        // server's own host instead of a wiremock host (it's always
        // 127.0.0.1:<port>), and verify zero requests were made.
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html></html>"))
            .expect(0)
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let host = url::Url::parse(&server.uri())
            .unwrap()
            .host_str()
            .unwrap()
            .to_string();
        let url = format!("{}/excluded", server.uri());
        let md = format!("[excluded]({} (preview))", url);
        let book = chapter_with(&md);

        let cfg = serde_json::json!({
            "cache-dir": ".cache/test6",
            "exclude-domains": [host],
        });
        let result = run_preprocessor(dir.path(), cfg, book);
        let content = first_chapter_content(&result);
        assert!(content.contains(&url));
        assert!(!content.contains("og-card"));

        server.verify().await;
    });
}

#[test]
fn shortcode_is_converted_into_og_card() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/short"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<html><head><meta property="og:title" content="Shortcode Page"></head></html>"#,
            ))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let url = format!("{}/short", server.uri());
        let md = format!("Intro text.\n\n{{{{#og {}}}}}\n\nOutro text.", url);
        let book = chapter_with(&md);

        let cfg = serde_json::json!({ "cache-dir": ".cache/test7", "allow-private-ips-for-tests": true });
        let result = run_preprocessor(dir.path(), cfg, book);
        let content = first_chapter_content(&result);
        assert!(content.contains("og-card"));
        assert!(content.contains("Shortcode Page"));
        assert!(content.contains("Intro text."));
        assert!(content.contains("Outro text."));
    });
}
