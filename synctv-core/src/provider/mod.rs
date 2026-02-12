// Media Provider System
//
// Three-tier architecture:
//
// Tier 1: synctv-media-providers (Pure provider HTTP clients)
//   - alist::AlistClient, bilibili::BilibiliClient, emby::EmbyClient
//   - Independent libraries with no MediaProvider dependency
//   - Can be used as provider_instances
//
// Tier 2: synctv-core/provider (MediaProvider adapters)
//   - AlistProvider, BilibiliProvider, EmbyProvider
//   - Call synctv-media-providers clients to implement MediaProvider trait
//
// Tier 3: synctv-core/service/providers_manager
//   - ProvidersManager - manages all MediaProvider instances
//   - Factory pattern for creating providers
//   - Integration with RemoteProviderManager

// Core traits and types
pub mod config;
pub mod context;
pub mod error;
pub mod provider_client;
pub mod registry;
pub mod traits;

// MediaProvider implementations (adapters)
pub mod alist;
pub mod bilibili;
pub mod direct_url;
pub mod emby;
pub mod rtmp;
pub mod live_proxy;

pub use config::*;
pub use context::*;
pub use error::*;
pub use registry::*;
pub use traits::*;

// Re-export providers
pub use alist::AlistProvider;
pub use bilibili::BilibiliProvider;
pub use direct_url::DirectUrlProvider;
pub use emby::EmbyProvider;
pub use rtmp::RtmpProvider;
pub use live_proxy::LiveProxyProvider;

/// Parse a `serde_json::Value` into a typed source config.
///
/// Common helper for provider `TryFrom<&Value>` implementations.
pub fn parse_source_config<T: serde::de::DeserializeOwned>(
    value: &serde_json::Value,
    provider_name: &str,
) -> std::result::Result<T, ProviderError> {
    serde_json::from_value(value.clone()).map_err(|e| {
        ProviderError::InvalidConfig(format!("Failed to parse {provider_name} source config: {e}"))
    })
}
