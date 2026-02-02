// SyncTV API Library
//
// Provides gRPC and HTTP API services for SyncTV

pub mod grpc;
pub mod http;
pub mod impls;
pub mod proto;
pub mod observability;

// Re-export commonly used types
pub use http::AppState;
