//! End-to-end tests covering the fetch -> extract -> cache pipeline and the
//! security/resilience behaviors described in the implementation plan.
//! Uses `wiremock` so no real network access is required.

use mdbook_og_cards::cache::{Cache, FetchOutcome};
use mdbook_og_cards::config::Config;
use mdbook_og_cards::extractor;
use mdbook_og_cards::fetch::fetch_html;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn test_config() -> Config {
    let mut cfg = Config::default();
    cfg.timeout = 2;
    cfg.max_redirects = 5;
    cfg.max_response_bytes = 1024 * 1024;
    // These tests talk to a local wiremock server, which lives on a
    // loopback address; real SSRF protection is exercised separately in
    // `blocks_requests_to_private_ip_targets` with the default (strict)
    // config below.
    cfg.allow_private_ips_for_tests = true;
    cfg
}

fn strict_config() -> Config {
    let mut cfg = test_config();
    cfg.allow_private_ips_for_tests = false;
    cfg
}

#[tokio::test]
async fn fetches_and_extracts_valid_og_metadata() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/page"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"<html><head>
                <meta property="og:title" content="Test Page">
                <meta property="og:description" content="A description">
                <meta property="og:image" content="https://cdn.example.com/img.png">
            </head></html>"#,
        ))
        .mount(&server)
        .await;

    let url = format!("{}/page", server.uri());
    let (html, final_url) = fetch_html(&test_config(), &url).await.unwrap();
    let meta = extractor::extract(&html, &final_url).unwrap();

    assert_eq!(meta.title.as_deref(), Some("Test Page"));
    assert_eq!(meta.description.as_deref(), Some("A description"));
    assert_eq!(meta.image.as_deref(), Some("https://cdn.example.com/img.png"));
}

#[tokio::test]
async fn returns_error_on_404() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let url = format!("{}/missing", server.uri());
    let result = fetch_html(&test_config(), &url).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn follows_redirect_and_extracts_final_metadata() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/old"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", "/new"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/new"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"<html><head><meta property="og:title" content="Final Page"></head></html>"#,
        ))
        .mount(&server)
        .await;

    let url = format!("{}/old", server.uri());
    let (html, final_url) = fetch_html(&test_config(), &url).await.unwrap();
    let meta = extractor::extract(&html, &final_url).unwrap();

    assert_eq!(meta.title.as_deref(), Some("Final Page"));
    assert!(final_url.ends_with("/new"));
}

#[tokio::test]
async fn times_out_on_slow_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_secs(5))
                .set_body_string("too slow"),
        )
        .mount(&server)
        .await;

    let mut cfg = test_config();
    cfg.timeout = 1;
    let url = format!("{}/slow", server.uri());
    let result = fetch_html(&cfg, &url).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn resolves_relative_image_after_redirect() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/r"))
        .respond_with(ResponseTemplate::new(301).insert_header("Location", "/final"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/final"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"<html><head>
                <meta property="og:title" content="Image Test">
                <meta property="og:image" content="/static/x.png">
            </head></html>"#,
        ))
        .mount(&server)
        .await;

    let url = format!("{}/r", server.uri());
    let (html, final_url) = fetch_html(&test_config(), &url).await.unwrap();
    let meta = extractor::extract(&html, &final_url).unwrap();

    let expected_image = format!("{}/static/x.png", server.uri());
    assert_eq!(meta.image.as_deref(), Some(expected_image.as_str()));
}

#[tokio::test]
async fn cached_url_does_not_trigger_a_second_http_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/once"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"<html><head><meta property="og:title" content="Cached"></head></html>"#,
        ))
        .expect(1) // wiremock will panic on drop if called more than once
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(dir.path(), 3600, 60).unwrap();
    let url = format!("{}/once", server.uri());

    // First call: cache miss, fetch, then populate cache.
    assert!(cache.get(&url).is_none());
    let (html, final_url) = fetch_html(&test_config(), &url).await.unwrap();
    let meta = extractor::extract(&html, &final_url).unwrap();
    cache.put(&url, FetchOutcome::Success(meta)).unwrap();

    // Second call: should be served from cache without hitting the server.
    match cache.get(&url) {
        Some(FetchOutcome::Success(m)) => assert_eq!(m.title.as_deref(), Some("Cached")),
        other => panic!("expected cache hit, got {other:?}"),
    }

    server.verify().await;
}

#[tokio::test]
async fn blocks_requests_to_private_ip_targets() {
    // 127.0.0.1 is always private/loopback regardless of any mock server,
    // so this must be rejected before any connection is attempted.
    let result = fetch_html(&strict_config(), "http://127.0.0.1:1/anything").await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("internal") || err.contains("refusing") || err.contains("loopback") || err.contains("reserved"),
        "unexpected error message: {err}"
    );
}

#[tokio::test]
async fn rejects_non_http_scheme_before_connecting() {
    let result = fetch_html(&strict_config(), "ftp://example.com/file").await;
    assert!(result.is_err());
}
