// Provider Instance Manager
//
// Manages provider instances with automatic fallback to local implementation.
// Supports both local (in-process) and remote (gRPC) provider instances.

use crate::models::ProviderInstance;
use crate::repository::ProviderInstanceRepository;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint};

/// Connected Provider Instance
///
/// Represents a provider instance with an active connection.
/// - Local instance: `channel` is None
/// - Remote instance: `channel` contains active gRPC Channel
pub struct ConnectedProviderInstance {
    pub config: ProviderInstance,
    pub channel: Option<Channel>,
}

impl ConnectedProviderInstance {
    /// Check if this is a local instance
    pub fn is_local(&self) -> bool {
        self.channel.is_none()
    }

    /// Check if this is a remote instance
    pub fn is_remote(&self) -> bool {
        self.channel.is_some()
    }
}

/// Provider Instance Manager
///
/// Global singleton that manages provider instances with automatic fallback.
///
/// Architecture:
/// - `local_instance`: Always-available local implementation (fallback)
/// - `instances`: HashMap of remote instances (indexed by name)
/// - `get(name)`: Returns remote if found, otherwise returns local (guaranteed to always return)
pub struct ProviderInstanceManager {
    /// Remote instances (indexed by name)
    instances: Arc<RwLock<HashMap<String, Arc<ConnectedProviderInstance>>>>,

    /// Local instance (fallback, always available)
    local_instance: Arc<ConnectedProviderInstance>,

    /// Repository for database operations
    repository: Arc<ProviderInstanceRepository>,
}

impl ProviderInstanceManager {
    /// Create a new ProviderInstanceManager with local instance fallback
    pub fn new(repository: Arc<ProviderInstanceRepository>) -> Self {
        // Create local instance (no gRPC connection needed)
        let local_instance = Arc::new(ConnectedProviderInstance {
            config: ProviderInstance {
                name: "local".to_string(),
                endpoint: "local://".to_string(),
                comment: Some("Local in-process implementation".to_string()),
                jwt_secret: None,
                custom_ca: None,
                timeout: "10s".to_string(),
                tls: false,
                insecure_tls: false,
                providers: vec![
                    "bilibili".to_string(),
                    "alist".to_string(),
                    "emby".to_string(),
                    "rtmp".to_string(),
                    "direct_url".to_string(),
                ],
                enabled: true,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            channel: None, // Local instance has no gRPC channel
        });

        Self {
            instances: Arc::new(RwLock::new(HashMap::new())),
            local_instance,
            repository,
        }
    }

    /// Initialize manager by loading all enabled instances from database
    ///
    /// Call this at application startup to establish gRPC connections
    /// to all configured remote provider instances.
    pub async fn init(&self) -> anyhow::Result<()> {
        tracing::info!("Initializing provider instance manager with local instance");

        // Load all enabled instances from database
        let configs = self.repository.get_all_enabled().await?;

        let mut instances = self.instances.write().await;
        let mut success_count = 0;
        let mut error_count = 0;

        for config in configs {
            match Self::create_connected_instance(config.clone()).await {
                Ok(instance) => {
                    instances.insert(config.name.clone(), Arc::new(instance));
                    tracing::info!("Loaded remote provider instance: {}", config.name);
                    success_count += 1;
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to load provider instance {}: {}",
                        config.name,
                        e
                    );
                    error_count += 1;
                }
            }
        }

        tracing::info!(
            "Provider instance manager initialized: {} remote instances loaded, {} failed",
            success_count,
            error_count
        );

        Ok(())
    }

    /// Create a connected provider instance with gRPC channel
    ///
    /// Establishes gRPC connection with configured TLS settings, timeout, and middleware.
    async fn create_connected_instance(
        config: ProviderInstance,
    ) -> anyhow::Result<ConnectedProviderInstance> {
        // Parse timeout
        let timeout = config
            .parse_timeout()
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        // Create endpoint
        let mut endpoint = Endpoint::from_shared(config.endpoint.clone())?
            .timeout(timeout)
            .tcp_keepalive(Some(Duration::from_secs(30)))
            .http2_keep_alive_interval(Duration::from_secs(30))
            .keep_alive_timeout(Duration::from_secs(10));

        // Configure TLS if enabled
        if config.tls {
            let mut tls_config = ClientTlsConfig::new();

            if config.insecure_tls {
                // Skip certificate verification (UNSAFE, development/testing only)
                tracing::warn!(
                    "Instance '{}' configured with insecure TLS (skips certificate verification)",
                    config.name
                );
                tls_config = tls_config.with_native_roots();
            } else {
                // Use custom CA certificate if provided
                if let Some(ref ca_pem) = config.custom_ca {
                    let cert = Certificate::from_pem(ca_pem);
                    tls_config = tls_config.ca_certificate(cert);
                } else {
                    // Use system CA certificates
                    tls_config = tls_config.with_native_roots();
                }
            }

            endpoint = endpoint.tls_config(tls_config)?;
        }

        // Connect to gRPC server
        let channel = endpoint.connect().await?;

        tracing::info!(
            "Established gRPC connection to {} (timeout: {:?}, TLS: {})",
            config.endpoint,
            timeout,
            config.tls
        );

        Ok(ConnectedProviderInstance {
            config,
            channel: Some(channel),
        })
    }

    /// Get a provider instance by name with automatic fallback
    ///
    /// **Guaranteed to always return an instance:**
    /// - If remote instance with `name` exists: returns remote instance
    /// - Otherwise: returns local instance (fallback)
    ///
    /// This ensures providers can always operate without needing to handle None.
    pub async fn get(&self, name: &str) -> Arc<ConnectedProviderInstance> {
        // Try to get remote instance
        if let Some(instance) = self.instances.read().await.get(name).cloned() {
            return instance;
        }

        // Fallback to local instance (always available)
        tracing::debug!(
            "Provider instance '{}' not found, using local instance",
            name
        );
        self.local_instance.clone()
    }

    /// Try to get a remote instance by name (no automatic fallback)
    ///
    /// Returns:
    /// - `Some(instance)` if remote instance exists
    /// - `None` if not found
    ///
    /// Use this when you need to explicitly check if a remote instance exists.
    /// For normal operations, prefer `get()` which guarantees a return value.
    pub async fn try_get(&self, name: &str) -> Option<Arc<ConnectedProviderInstance>> {
        self.instances.read().await.get(name).cloned()
    }

    /// Get the local instance
    ///
    /// Returns the always-available local (in-process) provider implementation.
    pub fn get_local(&self) -> Arc<ConnectedProviderInstance> {
        self.local_instance.clone()
    }

    /// List all remote instances
    pub async fn list(&self) -> Vec<Arc<ConnectedProviderInstance>> {
        self.instances.read().await.values().cloned().collect()
    }

    /// Find remote instances that support a specific provider type
    ///
    /// Returns all enabled remote instances that have `provider` in their supported list.
    pub async fn find_by_provider(&self, provider: &str) -> Vec<Arc<ConnectedProviderInstance>> {
        self.instances
            .read()
            .await
            .values()
            .filter(|instance| {
                instance.config.enabled && instance.config.supports_provider(provider)
            })
            .cloned()
            .collect()
    }

    /// Add a new provider instance
    ///
    /// 1. Creates gRPC connection
    /// 2. Saves to database
    /// 3. Adds to in-memory registry
    pub async fn add(&self, config: ProviderInstance) -> anyhow::Result<()> {
        // Validate name is not "local" (reserved)
        if config.name == "local" {
            anyhow::bail!("Instance name 'local' is reserved");
        }

        // Check if instance already exists
        if self.instances.read().await.contains_key(&config.name) {
            anyhow::bail!("Instance '{}' already exists", config.name);
        }

        // Create gRPC connection
        let instance = Self::create_connected_instance(config.clone()).await?;

        // Save to database
        self.repository.create(&config).await?;

        // Add to in-memory registry
        self.instances
            .write()
            .await
            .insert(config.name.clone(), Arc::new(instance));

        tracing::info!("Added provider instance: {}", config.name);
        Ok(())
    }

    /// Update an existing provider instance
    ///
    /// 1. Creates new gRPC connection
    /// 2. Updates database
    /// 3. Replaces in-memory instance (old connection closed automatically)
    pub async fn update(&self, config: ProviderInstance) -> anyhow::Result<()> {
        // Cannot update local instance
        if config.name == "local" {
            anyhow::bail!("Cannot update reserved 'local' instance");
        }

        // Create new gRPC connection
        let instance = Self::create_connected_instance(config.clone()).await?;

        // Update database
        self.repository.update(&config).await?;

        // Replace in-memory instance (old connection auto-closed via Arc drop)
        self.instances
            .write()
            .await
            .insert(config.name.clone(), Arc::new(instance));

        tracing::info!("Updated provider instance: {}", config.name);
        Ok(())
    }

    /// Delete a provider instance
    ///
    /// 1. Removes from database
    /// 2. Removes from in-memory registry (connection closed automatically)
    pub async fn delete(&self, name: &str) -> anyhow::Result<()> {
        // Cannot delete local instance
        if name == "local" {
            anyhow::bail!("Cannot delete reserved 'local' instance");
        }

        // Remove from database
        self.repository.delete(name).await?;

        // Remove from memory (connection auto-closed via Arc drop)
        self.instances.write().await.remove(name);

        tracing::info!("Deleted provider instance: {}", name);
        Ok(())
    }

    /// Enable a provider instance
    ///
    /// Re-establishes gRPC connection and adds to active registry.
    pub async fn enable(&self, name: &str) -> anyhow::Result<()> {
        // Update database
        self.repository.enable(name).await?;

        // Reload instance from database
        let config = self
            .repository
            .get_by_name(name)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found", name))?;

        // Create connection and add to registry
        let instance = Self::create_connected_instance(config.clone()).await?;
        self.instances
            .write()
            .await
            .insert(config.name.clone(), Arc::new(instance));

        tracing::info!("Enabled provider instance: {}", name);
        Ok(())
    }

    /// Disable a provider instance
    ///
    /// Removes from active registry and closes connection.
    pub async fn disable(&self, name: &str) -> anyhow::Result<()> {
        // Update database
        self.repository.disable(name).await?;

        // Remove from memory (connection auto-closed)
        self.instances.write().await.remove(name);

        tracing::info!("Disabled provider instance: {}", name);
        Ok(())
    }

    /// Health check all remote instances
    ///
    /// Returns a map of instance name to health status.
    pub async fn health_check(&self) -> HashMap<String, bool> {
        let instances = self.instances.read().await;
        let mut results = HashMap::new();

        for (name, instance) in instances.iter() {
            // Check if gRPC channel is ready
            let is_healthy = if instance.channel.is_some() {
                // For now, assume healthy if channel exists
                // TODO: Implement actual health check RPC
                true
            } else {
                false
            };

            results.insert(name.clone(), is_healthy);
        }

        // Local instance is always healthy
        results.insert("local".to_string(), true);

        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    #[tokio::test]
    async fn test_local_instance_creation() {
        // This test verifies the local instance is created correctly
        let pool = PgPool::connect_lazy("postgresql://test").unwrap();
        let repo = Arc::new(ProviderInstanceRepository::new(pool));
        let manager = ProviderInstanceManager::new(repo);

        let local = manager.get_local();
        assert!(local.is_local());
        assert_eq!(local.config.name, "local");
        assert!(local.config.supports_provider("bilibili"));
        assert!(local.config.supports_provider("alist"));
    }
}
