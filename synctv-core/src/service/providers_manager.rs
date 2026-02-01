//! Providers Manager
//!
//! Manages all MediaProvider instances with factory pattern.
//! Integrates with ProviderInstanceManager for local/remote provider client dispatch.

use crate::provider::{
    AlistProvider, BilibiliProvider, DirectUrlProvider, EmbyProvider, MediaProvider,
    RtmpProvider,
};
use crate::service::ProviderInstanceManager;
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Factory function type for creating MediaProvider instances
pub type ProviderFactory =
    Box<dyn Fn(&str, &Value, Arc<ProviderInstanceManager>) -> Result<Arc<dyn MediaProvider>> + Send + Sync>;

/// Providers Manager
///
/// Manages all MediaProvider instances using factory pattern.
/// Works with ProviderInstanceManager to dispatch calls to local or remote provider clients.
///
/// # Architecture
/// ```text
/// ProvidersManager
///   ├── Factories (registered for each provider type)
///   ├── Instances (created MediaProvider instances)
///   └── ProviderInstanceManager (for local/remote dispatch)
/// ```
pub struct ProvidersManager {
    /// Registered factory functions (provider_type → factory)
    factories: HashMap<String, ProviderFactory>,

    /// Created MediaProvider instances (provider_id → MediaProvider)
    media_providers: Arc<RwLock<HashMap<String, Arc<dyn MediaProvider>>>>,

    /// Provider instance manager (for local/remote dispatch)
    instance_manager: Arc<ProviderInstanceManager>,
}

impl ProvidersManager {
    /// Create a new ProvidersManager
    pub fn new(instance_manager: Arc<ProviderInstanceManager>) -> Self {
        let mut manager = Self {
            factories: HashMap::new(),
            media_providers: Arc::new(RwLock::new(HashMap::new())),
            instance_manager,
        };

        // Register all built-in providers
        manager.register_builtin_providers();

        manager
    }

    /// Get a reference to the provider instance manager
    pub fn instance_manager(&self) -> &Arc<ProviderInstanceManager> {
        &self.instance_manager
    }

    /// Register all built-in provider factories
    fn register_builtin_providers(&mut self) {
        // Alist factory
        self.register_factory(
            "alist",
            Box::new(|_instance_id, _config, instance_manager| {
                Ok(Arc::new(AlistProvider::new(instance_manager)))
            }),
        );

        // Bilibili factory
        self.register_factory(
            "bilibili",
            Box::new(|_instance_id, _config, instance_manager| {
                Ok(Arc::new(BilibiliProvider::new(instance_manager)))
            }),
        );

        // Emby factory
        self.register_factory(
            "emby",
            Box::new(|_instance_id, _config, instance_manager| {
                Ok(Arc::new(EmbyProvider::new(instance_manager)))
            }),
        );

        // RTMP factory
        self.register_factory(
            "rtmp",
            Box::new(|_instance_id, config, _instance_manager| {
                let base_url = config
                    .get("base_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("https://localhost:8080");

                Ok(Arc::new(RtmpProvider::new(base_url)))
            }),
        );

        // DirectUrl factory
        self.register_factory(
            "direct_url",
            Box::new(|_instance_id, _config, _instance_manager| {
                Ok(Arc::new(DirectUrlProvider::new()))
            }),
        );
    }

    /// Register a provider factory
    pub fn register_factory(&mut self, provider_type: &str, factory: ProviderFactory) {
        self.factories.insert(provider_type.to_string(), factory);
        tracing::debug!("Registered provider factory: {}", provider_type);
    }

    /// Create a provider instance
    ///
    /// # Arguments
    /// * `provider_type` - Type of provider ("alist", "bilibili", etc.)
    /// * `instance_id` - Unique instance identifier
    /// * `config` - Provider configuration (JSON)
    pub async fn create_provider(
        &self,
        provider_type: &str,
        instance_id: &str,
        config: &Value,
    ) -> Result<Arc<dyn MediaProvider>> {
        let factory = self
            .factories
            .get(provider_type)
            .ok_or_else(|| anyhow::anyhow!("Unknown provider type: {}", provider_type))?;

        let provider = factory(instance_id, config, self.instance_manager.clone())?;

        // Store in instances map
        self.media_providers
            .write()
            .await
            .insert(instance_id.to_string(), provider.clone());

        tracing::info!(
            "Created provider instance: {} (type: {})",
            instance_id,
            provider_type
        );

        Ok(provider)
    }

    /// Get a provider instance by ID
    pub async fn get_provider(&self, instance_id: &str) -> Option<Arc<dyn MediaProvider>> {
        self.media_providers.read().await.get(instance_id).cloned()
    }

    /// List all provider instances
    pub async fn list_providers(&self) -> Vec<Arc<dyn MediaProvider>> {
        self.media_providers.read().await.values().cloned().collect()
    }

    /// Remove a provider instance
    pub async fn remove_provider(&self, instance_id: &str) -> Option<Arc<dyn MediaProvider>> {
        self.media_providers.write().await.remove(instance_id)
    }

    /// Get or create a provider instance
    ///
    /// Returns existing instance if found, otherwise creates a new one
    pub async fn get_or_create_provider(
        &self,
        provider_type: &str,
        instance_id: &str,
        config: &Value,
    ) -> Result<Arc<dyn MediaProvider>> {
        if let Some(provider) = self.get_provider(instance_id).await {
            return Ok(provider);
        }

        self.create_provider(provider_type, instance_id, config)
            .await
    }

    /// Check if a provider type is registered
    pub fn has_factory(&self, provider_type: &str) -> bool {
        self.factories.contains_key(provider_type)
    }

    /// List all registered provider types
    pub fn list_provider_types(&self) -> Vec<String> {
        self.factories.keys().cloned().collect()
    }

    // Note: Service/route registration is now handled via extension traits
    // in synctv-api layer. See:
    // - synctv-api/src/http/provider_extensions.rs
    // - synctv-api/src/grpc/provider_extensions.rs
}

impl std::fmt::Debug for ProvidersManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let providers_count = self.media_providers.blocking_read().len();
        f.debug_struct("ProvidersManager")
            .field("factories_count", &self.factories.len())
            .field("media_providers_count", &providers_count)
            .field("instance_manager", &self.instance_manager)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::ProviderInstanceRepository;
    use sqlx::PgPool;

    #[tokio::test]
    async fn test_providers_manager_creation() {
        let pool = PgPool::connect_lazy("postgresql://test").unwrap();
        let repo = Arc::new(ProviderInstanceRepository::new(pool));
        let instance_manager = Arc::new(ProviderInstanceManager::new(repo));
        let manager = ProvidersManager::new(instance_manager);

        // Check that built-in providers are registered
        assert!(manager.has_factory("alist"));
        assert!(manager.has_factory("bilibili"));
        assert!(manager.has_factory("emby"));
        assert!(manager.has_factory("rtmp"));
        assert!(manager.has_factory("direct_url"));
        assert!(!manager.has_factory("unknown"));
    }

    #[tokio::test]
    async fn test_list_provider_types() {
        let pool = PgPool::connect_lazy("postgresql://test").unwrap();
        let repo = Arc::new(ProviderInstanceRepository::new(pool));
        let instance_manager = Arc::new(ProviderInstanceManager::new(repo));
        let manager = ProvidersManager::new(instance_manager);

        let types = manager.list_provider_types();
        assert!(types.contains(&"alist".to_string()));
        assert!(types.contains(&"bilibili".to_string()));
        assert_eq!(types.len(), 5); // alist, bilibili, emby, rtmp, direct_url
    }
}
