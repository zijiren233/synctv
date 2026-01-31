// Provider Registry
//
// Factory-based registry for managing provider instances

use super::{MediaProvider, ProviderError};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Provider factory function type
pub type ProviderFactory =
    Box<dyn Fn(&str, Value) -> Result<Arc<dyn MediaProvider>, ProviderError> + Send + Sync>;

/// Provider registry for managing instances
///
/// Uses factory pattern to create provider instances from configuration.
/// Each provider type registers a factory function.
pub struct ProviderRegistry {
    /// Registered provider factories by type name
    factories: HashMap<String, ProviderFactory>,

    /// Created provider instances by instance_id
    instances: HashMap<String, Arc<dyn MediaProvider>>,
}

impl ProviderRegistry {
    /// Create new empty registry
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
            instances: HashMap::new(),
        }
    }

    /// Register a provider factory
    ///
    /// # Example
    /// ```rust
    /// registry.register_factory("bilibili", Box::new(|instance_id, config| {
    ///     Ok(Arc::new(BilibiliProvider::new(instance_id, config)?))
    /// }));
    /// ```
    pub fn register_factory(&mut self, provider_type: &str, factory: ProviderFactory) {
        self.factories.insert(provider_type.to_string(), factory);
    }

    /// Create and register a provider instance
    ///
    /// # Arguments
    /// - `provider_type`: Type of provider (e.g., "bilibili", "alist")
    /// - `instance_id`: Unique instance ID (e.g., "bilibili_main", "alist_company")
    /// - `config`: Provider-specific configuration
    ///
    /// # Example
    /// ```rust
    /// let config = json!({
    ///     "base_url": "https://api.bilibili.com",
    ///     "cookies": "..."
    /// });
    /// registry.create_instance("bilibili", "bilibili_main", config)?;
    /// ```
    pub fn create_instance(
        &mut self,
        provider_type: &str,
        instance_id: &str,
        config: Value,
    ) -> Result<(), ProviderError> {
        let factory = self
            .factories
            .get(provider_type)
            .ok_or_else(|| ProviderError::InstanceNotFound(provider_type.to_string()))?;

        let instance = factory(instance_id, config)?;
        self.instances.insert(instance_id.to_string(), instance);

        Ok(())
    }

    /// Get provider instance by ID
    ///
    /// # Example
    /// ```rust
    /// let provider = registry.get_instance("bilibili_main")?;
    /// let result = provider.generate_playback(&ctx, &source_config).await?;
    /// ```
    pub fn get_instance(&self, instance_id: &str) -> Option<Arc<dyn MediaProvider>> {
        self.instances.get(instance_id).cloned()
    }

    /// List all registered instances
    pub fn list_instances(&self) -> Vec<String> {
        self.instances.keys().cloned().collect()
    }

    /// Remove an instance
    pub fn remove_instance(&mut self, instance_id: &str) -> bool {
        self.instances.remove(instance_id).is_some()
    }

    /// Build aggregated HTTP routes from all providers
    ///
    /// Collects HTTP routes from all registered provider instances.
    ///
    /// Note: This will be implemented when HTTP integration is added
    pub async fn build_routes(&self) -> Result<(), ProviderError> {
        for instance in self.instances.values() {
            if instance.needs_service_registration() {
                instance.register_http_routes().await?;
            }
        }
        Ok(())
    }

    /// Build aggregated gRPC services from all providers
    ///
    /// Collects gRPC services from all registered provider instances.
    ///
    /// Note: This will be implemented when gRPC integration is added
    pub async fn build_grpc_services(&self) -> Result<(), ProviderError> {
        for instance in self.instances.values() {
            if instance.needs_service_registration() {
                instance.register_grpc_service().await?;
            }
        }
        Ok(())
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockProvider {
        instance_id: String,
    }

    #[async_trait::async_trait]
    impl MediaProvider for MockProvider {
        fn name(&self) -> &'static str {
            "mock"
        }

        fn instance_id(&self) -> &str {
            &self.instance_id
        }

        fn capabilities(&self) -> super::super::ProviderCapabilities {
            super::super::ProviderCapabilities {
                can_parse: true,
                can_play: true,
                supports_subtitles: false,
                supports_quality: false,
                requires_auth: false,
            }
        }

        async fn generate_playback(
            &self,
            _ctx: &super::super::ProviderContext<'_>,
            _source_config: &Value,
        ) -> Result<super::super::PlaybackResult, ProviderError> {
            Ok(super::super::PlaybackResult {
                playback_infos: HashMap::new(),
                default_mode: "direct".to_string(),
                metadata: HashMap::new(),
            })
        }
    }

    #[test]
    fn test_registry_factory() {
        let mut registry = ProviderRegistry::new();

        // Register factory
        registry.register_factory(
            "mock",
            Box::new(|instance_id, _config| {
                Ok(Arc::new(MockProvider {
                    instance_id: instance_id.to_string(),
                }))
            }),
        );

        // Create instance
        registry
            .create_instance("mock", "mock_main", serde_json::json!({}))
            .unwrap();

        // Get instance
        let instance = registry.get_instance("mock_main").unwrap();
        assert_eq!(instance.instance_id(), "mock_main");
        assert_eq!(instance.name(), "mock");
    }
}
