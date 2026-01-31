//! Alist Vendor Client Error Types
//!
//! Pure vendor errors, no dependency on ProviderError

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AlistError {
    #[error("Network error: {0}")]
    Network(String),

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

impl From<reqwest::Error> for AlistError {
    fn from(err: reqwest::Error) -> Self {
        AlistError::Network(err.to_string())
    }
}

impl From<serde_json::Error> for AlistError {
    fn from(err: serde_json::Error) -> Self {
        AlistError::Parse(err.to_string())
    }
}

impl From<reqwest::header::InvalidHeaderValue> for AlistError {
    fn from(err: reqwest::header::InvalidHeaderValue) -> Self {
        AlistError::InvalidHeader(err.to_string())
    }
}
