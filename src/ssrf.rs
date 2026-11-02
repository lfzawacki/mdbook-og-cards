use anyhow::{anyhow, Result};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};

/// Validate that a URL is safe to fetch: http(s) scheme only, and every
/// resolved IP address for its host must be a routable, non-internal
/// address. This is deliberately conservative (new/unusual private ranges
/// are rejected) since we're fetching URLs that may come from anywhere.
pub fn validate_url(url: &url::Url) -> Result<()> {
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(anyhow!("unsupported URL scheme: {other}")),
    }

    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("URL has no host"))?;

    let port = url.port_or_known_default().unwrap_or(443);
    resolve_and_validate(host, port)
}

/// Resolve `host:port` and ensure none of the resulting IPs are private,
/// loopback, link-local, multicast, unspecified, or otherwise internal.
pub fn resolve_and_validate(host: &str, port: u16) -> Result<()> {
    let addrs: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|e| anyhow!("DNS resolution failed for {host}: {e}"))?
        .collect();

    if addrs.is_empty() {
        return Err(anyhow!("DNS resolution returned no addresses for {host}"));
    }

    for addr in &addrs {
        if is_blocked_ip(&addr.ip()) {
            return Err(anyhow!(
                "refusing to connect to internal/reserved address {} (resolved from {host})",
                addr.ip()
            ));
        }
    }

    Ok(())
}

fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => is_blocked_v6(v6),
    }
}

fn is_blocked_v4(ip: &Ipv4Addr) -> bool {
    if ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_documentation()
    {
        return true;
    }
    let o = ip.octets();
    // Carrier-grade NAT: 100.64.0.0/10
    if o[0] == 100 && (64..=127).contains(&o[1]) {
        return true;
    }
    // 0.0.0.0/8 "this network"
    if o[0] == 0 {
        return true;
    }
    false
}

fn is_blocked_v6(ip: &Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_multicast() || ip.is_unspecified() {
        return true;
    }
    let seg = ip.segments();
    // Unique local address fc00::/7
    if (seg[0] & 0xfe00) == 0xfc00 {
        return true;
    }
    // Link-local fe80::/10
    if (seg[0] & 0xffc0) == 0xfe80 {
        return true;
    }
    // IPv4-mapped / IPv4-compatible addresses: re-check as v4.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_blocked_v4(&v4);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_loopback() {
        assert!(is_blocked_ip(&"127.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip(&"::1".parse().unwrap()));
    }

    #[test]
    fn blocks_private_ranges() {
        assert!(is_blocked_ip(&"10.0.0.5".parse().unwrap()));
        assert!(is_blocked_ip(&"192.168.1.1".parse().unwrap()));
        assert!(is_blocked_ip(&"172.16.0.1".parse().unwrap()));
        assert!(is_blocked_ip(&"169.254.1.1".parse().unwrap()));
        assert!(is_blocked_ip(&"100.64.0.1".parse().unwrap()));
        assert!(is_blocked_ip(&"fc00::1".parse().unwrap()));
        assert!(is_blocked_ip(&"fe80::1".parse().unwrap()));
    }

    #[test]
    fn allows_public_ip() {
        assert!(!is_blocked_ip(&"93.184.216.34".parse().unwrap()));
        assert!(!is_blocked_ip(&"2606:2800:220:1:248:1893:25c8:1946".parse().unwrap()));
    }

    #[test]
    fn rejects_non_http_scheme() {
        let url = url::Url::parse("file:///etc/passwd").unwrap();
        assert!(validate_url(&url).is_err());
    }
}
