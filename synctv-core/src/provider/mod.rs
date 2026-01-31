// Media Provider System
//
// Three-tier architecture:
//
// Tier 1: synctv-providers (Pure vendor HTTP clients)
//   - alist::AlistClient, bilibili::BilibiliClient, emby::EmbyClient
//   - Independent libraries with no MediaProvider dependency
//   - Can be used as provider_instances
//
// Tier 2: synctv-core/provider (MediaProvider adapters)
//   - AlistProvider, BilibiliProvider, EmbyProvider
//   - Call synctv-providers clients to implement MediaProvider trait
//
// Tier 3: synctv-core/service/providers_manager
//   - ProvidersManager - manages all MediaProvider instances
//   - Factory pattern for creating providers
//   - Integration with ProviderInstanceManager

// Core traits and types
pub mod traits;
pub mod registry;
pub mod context;
pub mod error;
pub mod config;
pub mod provider_client;

// MediaProvider implementations (adapters)
pub mod alist;
pub mod bilibili;
pub mod emby;
pub mod rtmp;
pub mod direct_url;

pub use traits::*;
pub use registry::*;
pub use context::*;
pub use error::*;
pub use config::*;

// Re-export providers
pub use alist::AlistProvider;
pub use bilibili::BilibiliProvider;
pub use emby::EmbyProvider;
pub use rtmp::RtmpProvider;
pub use direct_url::DirectUrlProvider;
