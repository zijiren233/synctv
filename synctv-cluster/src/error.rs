//! Error types for cluster module

use thiserror::Error;

/// Cluster error types
#[derive(Debug, Error)]
pub enum Error {
    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("RPC error: {0}")]
    Rpc(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Redis error: {0}")]
    Redis(String),

    #[error("Timeout: {0}")]
    Timeout(String),
}

/// Result type for cluster operations
pub type Result<T> = std::result::Result<T, Error>;
