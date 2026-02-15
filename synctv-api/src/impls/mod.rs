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
pub mod notification;
pub mod providers;

// Re-export for convenience
pub use admin::AdminApiImpl;
pub use client::ClientApiImpl;
pub use email::EmailApiImpl;
pub use messaging::{StreamMessageHandler, MessageSender, ProtoCodec};
pub use notification::NotificationApiImpl;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_error_not_found() {
        assert!(matches!(classify_error("User not found"), ErrorKind::NotFound));
        assert!(matches!(classify_error("Room Not Found"), ErrorKind::NotFound));
        assert!(matches!(classify_error("resource NOT FOUND"), ErrorKind::NotFound));
    }

    #[test]
    fn test_classify_error_unauthenticated() {
        assert!(matches!(classify_error("Unauthenticated"), ErrorKind::Unauthenticated));
        assert!(matches!(classify_error("invalid token"), ErrorKind::Unauthenticated));
        assert!(matches!(classify_error("Token expired"), ErrorKind::Unauthenticated));
        assert!(matches!(classify_error("Not authenticated"), ErrorKind::Unauthenticated));
    }

    #[test]
    fn test_classify_error_permission_denied() {
        assert!(matches!(classify_error("Permission denied"), ErrorKind::PermissionDenied));
        assert!(matches!(classify_error("Forbidden access"), ErrorKind::PermissionDenied));
        assert!(matches!(classify_error("Operation not allowed"), ErrorKind::PermissionDenied));
        assert!(matches!(classify_error("User is banned"), ErrorKind::PermissionDenied));
    }

    #[test]
    fn test_classify_error_already_exists() {
        assert!(matches!(classify_error("User already exists"), ErrorKind::AlreadyExists));
        assert!(matches!(classify_error("Username already taken"), ErrorKind::AlreadyExists));
        assert!(matches!(classify_error("Email already registered"), ErrorKind::AlreadyExists));
    }

    #[test]
    fn test_classify_error_invalid_argument() {
        assert!(matches!(classify_error("Invalid email format"), ErrorKind::InvalidArgument));
        assert!(matches!(classify_error("Password too short"), ErrorKind::InvalidArgument));
        assert!(matches!(classify_error("Username too long"), ErrorKind::InvalidArgument));
        assert!(matches!(classify_error("Field cannot be empty"), ErrorKind::InvalidArgument));
        assert!(matches!(classify_error("Too many rooms"), ErrorKind::InvalidArgument));
        assert!(matches!(classify_error("Email required"), ErrorKind::InvalidArgument));
        assert!(matches!(classify_error("Password must be alphanumeric"), ErrorKind::InvalidArgument));
    }

    #[test]
    fn test_classify_error_internal() {
        assert!(matches!(classify_error("Something went wrong"), ErrorKind::Internal));
        assert!(matches!(classify_error("Database connection failed"), ErrorKind::Internal));
        assert!(matches!(classify_error("Unexpected error"), ErrorKind::Internal));
    }

    #[test]
    fn test_classify_error_case_insensitive() {
        assert!(matches!(classify_error("NOT FOUND"), ErrorKind::NotFound));
        assert!(matches!(classify_error("PERMISSION denied"), ErrorKind::PermissionDenied));
        assert!(matches!(classify_error("INVALID token"), ErrorKind::Unauthenticated));
    }
}
