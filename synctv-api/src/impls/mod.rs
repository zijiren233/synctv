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
