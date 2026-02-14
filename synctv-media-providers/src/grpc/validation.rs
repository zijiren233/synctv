//! Shared validation utilities for gRPC server layers
//!
//! Provides SSRF-safe host validation and common field validators.

use std::net::IpAddr;
use tonic::Status;

/// Blocked private/internal hostnames (case-insensitive check)
const BLOCKED_HOSTNAMES: &[&str] = &[
    "localhost",
    "metadata.google.internal",
];

/// Blocked hostname suffixes (case-insensitive check)
const BLOCKED_HOSTNAME_SUFFIXES: &[&str] = &[
    ".internal",
    ".local",
];

/// Check if an IP address is in a private/reserved range that should be blocked.
fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()           // 127.0.0.0/8
            || v4.is_private()         // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
            || v4.is_link_local()      // 169.254.0.0/16
            || v4.is_unspecified()     // 0.0.0.0
            || v4.is_broadcast()       // 255.255.255.255
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()           // ::1
            || v6.is_unspecified()     // ::
            // fc00::/7 (unique local addresses)
            || (v6.segments()[0] & 0xfe00) == 0xfc00
        }
    }
}

/// Validate that a host string is a non-empty, valid URL with SSRF protections.
///
/// Checks:
/// - URL is parseable
/// - Scheme is http or https only
/// - Host is not a private IP range
/// - Host is not a known internal hostname
pub fn validate_host(host: &str) -> Result<(), Status> {
    if host.is_empty() {
        return Err(Status::invalid_argument("host must not be empty"));
    }

    let parsed = url::Url::parse(host)
        .map_err(|e| Status::invalid_argument(format!("invalid host URL: {e}")))?;

    // Verify scheme is http or https only
    match parsed.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(Status::invalid_argument(format!(
                "unsupported URL scheme: {scheme} (only http and https are allowed)"
            )));
        }
    }

    let url_host = parsed
        .host_str()
        .ok_or_else(|| Status::invalid_argument("host URL must contain a hostname"))?;

    let host_lower = url_host.to_lowercase();

    // Block known internal hostnames
    for blocked in BLOCKED_HOSTNAMES {
        if host_lower == *blocked {
            return Err(Status::invalid_argument(format!(
                "host URL must not target internal address: {url_host}"
            )));
        }
    }
    for suffix in BLOCKED_HOSTNAME_SUFFIXES {
        if host_lower.ends_with(suffix) {
            return Err(Status::invalid_argument(format!(
                "host URL must not target internal address: {url_host}"
            )));
        }
    }

    // Try to parse as IP address and block private ranges
    if let Ok(ip) = url_host.parse::<IpAddr>() {
        if is_blocked_ip(ip) {
            return Err(Status::invalid_argument(format!(
                "host URL must not target private/reserved IP: {url_host}"
            )));
        }
    }

    // Also handle bracket-wrapped IPv6 like [::1]
    if url_host.starts_with('[') && url_host.ends_with(']') {
        if let Ok(ip) = url_host[1..url_host.len() - 1].parse::<IpAddr>() {
            if is_blocked_ip(ip) {
                return Err(Status::invalid_argument(format!(
                    "host URL must not target private/reserved IP: {url_host}"
                )));
            }
        }
    }

    Ok(())
}

/// Validate that a required string field is non-empty.
pub fn validate_required(field_name: &str, value: &str) -> Result<(), Status> {
    if value.is_empty() {
        return Err(Status::invalid_argument(format!(
            "{field_name} must not be empty"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_hosts() {
        assert!(validate_host("https://example.com").is_ok());
        assert!(validate_host("http://my-alist.example.com:5244").is_ok());
        assert!(validate_host("https://emby.myserver.org/emby").is_ok());
    }

    #[test]
    fn test_blocked_schemes() {
        assert!(validate_host("ftp://example.com").is_err());
        assert!(validate_host("file:///etc/passwd").is_err());
        assert!(validate_host("gopher://evil.com").is_err());
    }

    #[test]
    fn test_blocked_private_ips() {
        assert!(validate_host("http://127.0.0.1").is_err());
        assert!(validate_host("http://10.0.0.1").is_err());
        assert!(validate_host("http://172.16.0.1").is_err());
        assert!(validate_host("http://192.168.1.1").is_err());
        assert!(validate_host("http://169.254.1.1").is_err());
        assert!(validate_host("http://0.0.0.0").is_err());
    }

    #[test]
    fn test_blocked_hostnames() {
        assert!(validate_host("http://localhost").is_err());
        assert!(validate_host("http://LOCALHOST").is_err());
        assert!(validate_host("http://metadata.google.internal").is_err());
        assert!(validate_host("http://something.internal").is_err());
        assert!(validate_host("http://myhost.local").is_err());
    }

    #[test]
    fn test_empty_host() {
        assert!(validate_host("").is_err());
    }

    #[test]
    fn test_invalid_url() {
        assert!(validate_host("not-a-url").is_err());
    }
}
