//! Emby/Jellyfin Provider Client Error Types

use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmbyError {
    #[error("Network error: {0}")]
    Network(String),

    #[error("HTTP error {status} for {url}")]
    Http { status: reqwest::StatusCode, url: String },

    #[error("API error: {0}")]
    Api(String),

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
}

/// Check HTTP response status before processing body.
pub fn check_response(resp: reqwest::Response) -> Result<reqwest::Response, EmbyError> {
    let status = resp.status();
    if status.is_client_error() || status.is_server_error() {
        return Err(EmbyError::Http {
            status,
            url: resp.url().to_string(),
        });
    }
    Ok(resp)
}

impl From<reqwest::Error> for EmbyError {
    fn from(err: reqwest::Error) -> Self {
        Self::Network(err.to_string())
    }
}

impl From<serde_json::Error> for EmbyError {
    fn from(err: serde_json::Error) -> Self {
        Self::Parse(err.to_string())
    }
}

impl From<reqwest::header::InvalidHeaderValue> for EmbyError {
    fn from(err: reqwest::header::InvalidHeaderValue) -> Self {
        Self::InvalidHeader(err.to_string())
    }
}
