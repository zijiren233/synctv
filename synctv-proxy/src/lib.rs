//! Shared media proxy utilities
//!
//! Provides reusable functions for proxying media streams and rewriting M3U8
//! playlists.  Used by per-provider proxy routes in `synctv-api`.

pub mod mpd;

use std::collections::HashMap;

use axum::{
    body::Body,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

/// Configuration for a single proxy fetch.
pub struct ProxyConfig<'a> {
    /// The remote URL to fetch.
    pub url: &'a str,
    /// Extra headers the provider requires (e.g. Referer, cookies).
    pub provider_headers: &'a HashMap<String, String>,
    /// Original client request headers to forward.
    pub client_headers: &'a HeaderMap,
}

/// Fetch a remote URL and return the response with CORS headers.
pub async fn proxy_fetch_and_forward(cfg: ProxyConfig<'_>) -> Result<Response, anyhow::Error> {
    let client = reqwest::Client::builder()
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {e}"))?;

    let mut request = client.get(cfg.url);

    // Forward relevant client headers
    for (name, value) in cfg.client_headers {
        if matches!(
            name.as_str(),
            "host" | "connection" | "accept-encoding" | "content-length" | "transfer-encoding"
        ) {
            continue;
        }
        if let Ok(v) = value.to_str() {
            request = request.header(name.as_str(), v);
        }
    }

    // Apply provider-required headers
    for (name, value) in cfg.provider_headers {
        request = request.header(name.as_str(), value.as_str());
    }

    // Default User-Agent if provider didn't set one
    if !cfg.provider_headers.contains_key("User-Agent") {
        request = request.header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
        );
    }

    // Default Referer from source URL if provider didn't set one
    if !cfg.provider_headers.contains_key("Referer") {
        if let Ok(parsed) = url::Url::parse(cfg.url) {
            let referer = format!(
                "{}://{}{}",
                parsed.scheme(),
                parsed.host_str().unwrap_or(""),
                parsed.path()
            );
            request = request.header("Referer", referer);
        }
    }

    let proxy_response = request
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Proxy request failed: {e}"))?;

    let status = proxy_response.status();
    let response_headers = proxy_response.headers().clone();

    let body_bytes = proxy_response
        .bytes()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read response body: {e}"))?;

    let mut builder = Response::builder().status(status);

    for (name, value) in &response_headers {
        if matches!(
            name.as_str(),
            "connection" | "transfer-encoding" | "content-encoding" | "content-length"
        ) {
            continue;
        }
        if let Ok(v) = value.to_str() {
            builder = builder.header(name.as_str(), v);
        }
    }

    builder = builder.header("Cache-Control", "no-cache");
    builder = builder.header("Pragma", "no-cache");

    builder
        .body(Body::from(body_bytes))
        .map_err(|e| anyhow::anyhow!("Failed to build response: {e}"))
}

/// Fetch a remote M3U8, rewrite its URLs so segments proxy through
/// `proxy_base`, and return the rewritten content.
pub async fn proxy_m3u8_and_rewrite(
    url: &str,
    provider_headers: &HashMap<String, String>,
    proxy_base: &str,
) -> Result<Response, anyhow::Error> {
    let client = reqwest::Client::builder()
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {e}"))?;

    let mut request = client.get(url);

    for (name, value) in provider_headers {
        request = request.header(name.as_str(), value.as_str());
    }

    if !provider_headers.contains_key("User-Agent") {
        request = request.header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
        );
    }

    if !provider_headers.contains_key("Referer") {
        if let Ok(parsed) = url::Url::parse(url) {
            let referer = format!(
                "{}://{}{}",
                parsed.scheme(),
                parsed.host_str().unwrap_or(""),
                parsed.path()
            );
            request = request.header("Referer", referer);
        }
    }

    let proxy_response = request
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("M3U8 proxy request failed: {e}"))?;

    if !proxy_response.status().is_success() {
        return Err(anyhow::anyhow!(
            "Remote M3U8 returned status {}",
            proxy_response.status()
        ));
    }

    let m3u8_text = proxy_response
        .text()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read M3U8 body: {e}"))?;

    let rewritten = rewrite_m3u8(&m3u8_text, url, proxy_base);

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/vnd.apple.mpegurl")
        .header("Cache-Control", "no-cache")
        .body(Body::from(rewritten))
        .map_err(|e| anyhow::anyhow!("Failed to build M3U8 response: {e}"))
}

/// Preflight handler suitable for `OPTIONS` routes.
#[allow(clippy::unused_async)]
pub async fn proxy_options_preflight() -> impl IntoResponse {
    StatusCode::NO_CONTENT
}

// ------------------------------------------------------------------
// M3U8 rewriting helpers
// ------------------------------------------------------------------

/// Rewrite URLs inside an M3U8 playlist so they proxy through the server.
fn rewrite_m3u8(m3u8: &str, source_url: &str, proxy_base: &str) -> String {
    let base = url::Url::parse(source_url).ok();
    let mut output = String::with_capacity(m3u8.len());

    for line in m3u8.lines() {
        if line.starts_with('#') {
            let rewritten_line = rewrite_uri_attribute(line, base.as_ref(), proxy_base);
            output.push_str(&rewritten_line);
        } else {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                output.push_str(line);
            } else {
                let absolute = make_absolute(trimmed, base.as_ref());
                let proxied = format!("{}?url={}", proxy_base, percent_encode(&absolute));
                output.push_str(&proxied);
            }
        }
        output.push('\n');
    }

    output
}

/// Resolve a possibly-relative URL to absolute using the given base URL.
fn make_absolute(raw: &str, base: Option<&url::Url>) -> String {
    if raw.starts_with("http://") || raw.starts_with("https://") {
        return raw.to_string();
    }
    if let Some(base) = base {
        if let Ok(joined) = base.join(raw) {
            return joined.to_string();
        }
    }
    raw.to_string()
}

/// Rewrite any `URI="..."` values found in an M3U8 tag line.
fn rewrite_uri_attribute(line: &str, base: Option<&url::Url>, proxy_base: &str) -> String {
    let pattern = "URI=\"";
    let mut result = String::with_capacity(line.len());
    let mut remaining = line;

    while let Some(start) = remaining.find(pattern) {
        result.push_str(&remaining[..start + pattern.len()]);
        remaining = &remaining[start + pattern.len()..];

        if let Some(end) = remaining.find('"') {
            let uri = &remaining[..end];
            let absolute = make_absolute(uri, base);
            let proxied = format!("{}?url={}", proxy_base, percent_encode(&absolute));
            result.push_str(&proxied);
            result.push('"');
            remaining = &remaining[end + 1..];
        } else {
            result.push_str(remaining);
            remaining = "";
        }
    }

    result.push_str(remaining);
    result
}

/// Minimal percent-encoding for URL query parameter values.
#[must_use] 
pub fn percent_encode(input: &str) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(input.len() * 2);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                let _ = write!(result, "%{byte:02X}");
            }
        }
    }
    result
}

// ------------------------------------------------------------------
// Internal helpers
// ------------------------------------------------------------------

