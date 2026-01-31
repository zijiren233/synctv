// SyncTV Provider Clients
//
// This crate contains pure HTTP client implementations and gRPC servers for various media providers.
// These clients are independent of the MediaProvider trait and can be used standalone
// or as provider_instances in the SyncTV system.
//
// Architecture:
// - synctv-providers: Pure HTTP clients + gRPC servers (Alist, Bilibili, Emby)
// - synctv-core/provider: MediaProvider trait implementations (adapters calling these clients)
// - synctv-core/service: ProvidersManager for managing provider instances

// HTTP clients (no MediaProvider dependency)
pub mod alist;
pub mod bilibili;
pub mod emby;

// gRPC servers (wrap HTTP clients)
pub mod grpc;

// Re-export client types for convenience
pub use alist::{AlistClient, AlistError};
pub use bilibili::{BilibiliClient, BilibiliError};
pub use emby::{EmbyClient, EmbyError};
