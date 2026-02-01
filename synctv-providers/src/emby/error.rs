//! Emby/Jellyfin Provider Client Error Types

use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmbyError {
    #[error("Network error: {0}")]
    Network(String),

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

impl From<reqwest::Error> for EmbyError {
    fn from(err: reqwest::Error) -> Self {
        EmbyError::Network(err.to_string())
    }
}

impl From<serde_json::Error> for EmbyError {
    fn from(err: serde_json::Error) -> Self {
        EmbyError::Parse(err.to_string())
    }
}

impl From<reqwest::header::InvalidHeaderValue> for EmbyError {
    fn from(err: reqwest::header::InvalidHeaderValue) -> Self {
        EmbyError::InvalidHeader(err.to_string())
    }
}
