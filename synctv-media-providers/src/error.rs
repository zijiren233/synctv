//! Shared provider client error types
//!
//! Common error enum and utilities used by all provider clients (Alist, Bilibili, Emby).

use thiserror::Error;

/// Maximum response body size for provider HTTP calls (16 MB).
/// Prevents OOM from malicious or misconfigured upstream servers.
pub const MAX_RESPONSE_SIZE: usize = 16 * 1024 * 1024;

/// Common error type for all provider HTTP clients.
#[derive(Debug, Error)]
pub enum ProviderClientError {
    #[error("Network error: {0}")]
    Network(String),

    #[error("HTTP error {status} for {url}")]
    Http { status: reqwest::StatusCode, url: String },

    #[error("API error (code {code}): {message}")]
    Api { code: u64, message: String },

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("Invalid header value: {0}")]
    InvalidHeader(String),

    #[error("Not implemented: {0}")]
    NotImplemented(String),

    #[error("Response too large ({size} bytes, max {MAX_RESPONSE_SIZE})")]
    ResponseTooLarge { size: u64 },
}

/// Read a response body with size limit and deserialize as JSON.
///
/// Checks `Content-Length` hint first (if available), then enforces the
/// limit on the actual body bytes before deserializing.
pub async fn json_with_limit<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, ProviderClientError> {
    if let Some(cl) = response.content_length() {
        if cl as usize > MAX_RESPONSE_SIZE {
            return Err(ProviderClientError::ResponseTooLarge { size: cl });
        }
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_RESPONSE_SIZE {
        return Err(ProviderClientError::ResponseTooLarge { size: bytes.len() as u64 });
    }
    serde_json::from_slice(&bytes).map_err(Into::into)
}

/// Check HTTP response status before processing body.
pub fn check_response(resp: reqwest::Response) -> Result<reqwest::Response, ProviderClientError> {
    let status = resp.status();
    if status.is_client_error() || status.is_server_error() {
        return Err(ProviderClientError::Http {
            status,
            url: resp.url().to_string(),
        });
    }
    Ok(resp)
}

impl From<reqwest::Error> for ProviderClientError {
    fn from(err: reqwest::Error) -> Self {
        Self::Network(err.to_string())
    }
}

impl From<serde_json::Error> for ProviderClientError {
    fn from(err: serde_json::Error) -> Self {
        Self::Parse(err.to_string())
    }
}

impl From<reqwest::header::InvalidHeaderValue> for ProviderClientError {
    fn from(err: reqwest::header::InvalidHeaderValue) -> Self {
        Self::InvalidHeader(err.to_string())
    }
}
