use crate::config::Config;
use crate::ssrf;
use anyhow::{anyhow, Context, Result};
use reqwest::redirect::Policy;
use std::time::Duration;
use url::Url;

/// Fetch a URL's HTML body, enforcing SSRF protections (scheme + DNS/IP
/// validation on every hop), a response size cap, and a request timeout.
/// Redirects are followed manually (reqwest's automatic redirect following
/// is disabled) so each hop can be re-validated before connecting.
pub async fn fetch_html(config: &Config, start_url: &str) -> Result<(String, String)> {
    let client = reqwest::Client::builder()
        .user_agent(&config.user_agent)
        .redirect(Policy::none())
        .timeout(Duration::from_secs(config.timeout))
        .build()
        .context("building HTTP client")?;

    let mut current = Url::parse(start_url).context("parsing URL")?;

    for _ in 0..=config.max_redirects {
        if !config.allow_private_ips_for_tests {
            ssrf::validate_url(&current)?;
        }

        let resp = client
            .get(current.clone())
            .send()
            .await
            .with_context(|| format!("requesting {current}"))?;

        let status = resp.status();
        if status.is_redirection() {
            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| anyhow!("redirect response {status} missing Location header"))?;
            current = current
                .join(location)
                .with_context(|| format!("resolving redirect Location: {location}"))?;
            continue;
        }

        if !status.is_success() {
            return Err(anyhow!("request to {current} failed with status {status}"));
        }

        if let Some(len) = resp.content_length() {
            if len > config.max_response_bytes {
                return Err(anyhow!(
                    "response body too large ({len} bytes > limit {})",
                    config.max_response_bytes
                ));
            }
        }

        let final_url = current.to_string();
        let body = read_capped(resp, config.max_response_bytes).await?;
        return Ok((body, final_url));
    }

    Err(anyhow!(
        "too many redirects (limit {})",
        config.max_redirects
    ))
}

async fn read_capped(resp: reqwest::Response, limit: u64) -> Result<String> {
    let body = resp.bytes().await.context("reading response body")?;
    if body.len() as u64 > limit {
        return Err(anyhow!(
            "response body exceeded size limit of {limit} bytes"
        ));
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}
