//! Bilibili Vendor Client Error Types

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BilibiliError {
    #[error("Network error: {0}")]
    Network(String),

    #[error("API error: {0}")]
    Api(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Invalid BVID/EPID: {0}")]
    InvalidId(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("Not implemented: {0}")]
    NotImplemented(String),
}

impl From<reqwest::Error> for BilibiliError {
    fn from(err: reqwest::Error) -> Self {
        BilibiliError::Network(err.to_string())
    }
}

impl From<serde_json::Error> for BilibiliError {
    fn from(err: serde_json::Error) -> Self {
        BilibiliError::Parse(err.to_string())
    }
}
