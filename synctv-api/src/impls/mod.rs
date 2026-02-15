//! Unified API Implementation Layer
//!
//! This module contains the actual implementation of all APIs.
//! Both HTTP and gRPC handlers are thin wrappers that call these implementations.
//!
//! All methods use grpc-generated types for parameters and return values.

pub mod admin;
pub mod client;
pub mod email;
pub mod messaging;
pub mod providers;

// Re-export for convenience
pub use admin::AdminApiImpl;
pub use client::ClientApiImpl;
pub use email::EmailApiImpl;
pub use messaging::{StreamMessageHandler, MessageSender, ProtoCodec};
pub use providers::{AlistApiImpl, BilibiliApiImpl, EmbyApiImpl};

/// Shared error classification for impls-layer `String` errors.
///
/// Maps keyword patterns in error strings to semantic error categories.
/// Used by both HTTP and gRPC error mapping functions to ensure consistent
/// behavior across transports.
///
/// Once the impls layer migrates to typed errors (`synctv_core::Error`),
/// this function should be replaced by `From` impls.
pub enum ErrorKind {
    NotFound,
    Unauthenticated,
    PermissionDenied,
    AlreadyExists,
    InvalidArgument,
    Internal,
}

/// Classify an impls-layer error string into a semantic error kind.
pub fn classify_error(err: &str) -> ErrorKind {
    let lower = err.to_lowercase();
    if lower.contains("not found") {
        ErrorKind::NotFound
    } else if lower.contains("unauthenticated") || lower.contains("invalid token")
        || lower.contains("token expired") || lower.contains("not authenticated")
    {
        ErrorKind::Unauthenticated
    } else if lower.contains("permission") || lower.contains("forbidden")
        || lower.contains("not allowed") || lower.contains("banned")
    {
        ErrorKind::PermissionDenied
    } else if lower.contains("already exists") || lower.contains("already taken")
        || lower.contains("already registered")
    {
        ErrorKind::AlreadyExists
    } else if lower.contains("invalid") || lower.contains("too short") || lower.contains("too long")
        || lower.contains("cannot be empty") || lower.contains("too many")
        || lower.contains("required") || lower.contains("must be")
    {
        ErrorKind::InvalidArgument
    } else {
        ErrorKind::Internal
    }
}
