//! Alist Provider Client Error Types
//!
//! Pure provider errors, no dependency on `ProviderError`

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AlistError {
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
}

/// Check HTTP response status before processing body.
pub fn check_response(resp: reqwest::Response) -> Result<reqwest::Response, AlistError> {
    let status = resp.status();
    if status.is_client_error() || status.is_server_error() {
        return Err(AlistError::Http {
            status,
            url: resp.url().to_string(),
        });
    }
    Ok(resp)
}

impl From<reqwest::Error> for AlistError {
    fn from(err: reqwest::Error) -> Self {
        Self::Network(err.to_string())
    }
}

impl From<serde_json::Error> for AlistError {
    fn from(err: serde_json::Error) -> Self {
        Self::Parse(err.to_string())
    }
}

impl From<reqwest::header::InvalidHeaderValue> for AlistError {
    fn from(err: reqwest::header::InvalidHeaderValue) -> Self {
        Self::InvalidHeader(err.to_string())
    }
}
