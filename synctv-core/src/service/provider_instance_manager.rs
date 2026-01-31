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

/// Provider Instance Manager
///
/// Manages remote provider instances (gRPC connections).
/// When no remote instance is found, providers fallback to singleton local clients.
///
/// Architecture:
/// - `instances`: HashMap of remote gRPC channels (indexed by name)
/// - `get(name)`: Returns Some(channel) if remote instance found, None otherwise
#[derive(Debug)]
pub struct ProviderInstanceManager {
    /// Remote instances (indexed by name → gRPC Channel)
    instances: Arc<RwLock<HashMap<String, Channel>>>,

    /// Repository for database operations
    repository: Arc<ProviderInstanceRepository>,
}

impl ProviderInstanceManager {
    /// Create a new ProviderInstanceManager
    pub fn new(repository: Arc<ProviderInstanceRepository>) -> Self {
        Self {
            instances: Arc::new(RwLock::new(HashMap::new())),
            repository,
        }
    }

    /// Initialize manager by loading all enabled instances from database
    ///
    /// Call this at application startup to establish gRPC connections
    /// to all configured remote provider instances.
    pub async fn init(&self) -> anyhow::Result<()> {
        tracing::info!("Initializing provider instance manager");

        // Load all enabled instances from database
        let configs = self.repository.get_all_enabled().await?;

        let mut instances = self.instances.write().await;
        let mut success_count = 0;
        let mut error_count = 0;

        for config in configs {
            match Self::create_grpc_channel(&config).await {
                Ok(channel) => {
                    instances.insert(config.name.clone(), channel);
                    tracing::info!("Loaded remote provider instance: {}", config.name);
                    success_count += 1;
                }
                Err(e) => {
                    tracing::error!("Failed to load provider instance {}: {}", config.name, e);
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

    /// Create a gRPC channel for the given provider instance
    ///
    /// Establishes gRPC connection with configured TLS settings, timeout, and middleware.
    async fn create_grpc_channel(config: &ProviderInstance) -> anyhow::Result<Channel> {
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

        Ok(channel)
    }

    /// Get a remote provider instance channel by name
    ///
    /// Returns:
    /// - `Some(channel)` if remote instance exists
    /// - `None` if not found (caller should fallback to singleton local client)
    pub async fn get(&self, name: &str) -> Option<Channel> {
        self.instances.read().await.get(name).cloned()
    }

    /// List all remote instance names
    pub async fn list(&self) -> Vec<String> {
        self.instances.read().await.keys().cloned().collect()
    }

    /// Get all provider instances with full metadata
    pub async fn get_all_instances(&self) -> anyhow::Result<Vec<ProviderInstance>> {
        self.repository.get_all().await.map_err(|e| anyhow::anyhow!(e))
    }

    /// Add a new provider instance
    ///
    /// 1. Creates gRPC connection
    /// 2. Saves to database
    /// 3. Adds to in-memory registry
    pub async fn add(&self, config: ProviderInstance) -> anyhow::Result<()> {
        // Check if instance already exists
        if self.instances.read().await.contains_key(&config.name) {
            anyhow::bail!("Instance '{}' already exists", config.name);
        }

        // Create gRPC connection
        let channel = Self::create_grpc_channel(&config).await?;

        // Save to database
        self.repository.create(&config).await?;

        // Add to in-memory registry
        self.instances
            .write()
            .await
            .insert(config.name.clone(), channel);

        tracing::info!("Added provider instance: {}", config.name);
        Ok(())
    }

    /// Update an existing provider instance
    ///
    /// 1. Creates new gRPC connection
    /// 2. Updates database
    /// 3. Replaces in-memory channel (old connection closed automatically)
    pub async fn update(&self, config: ProviderInstance) -> anyhow::Result<()> {
        // Create new gRPC connection
        let channel = Self::create_grpc_channel(&config).await?;

        // Update database
        self.repository.update(&config).await?;

        // Replace in-memory channel (old connection auto-closed)
        self.instances
            .write()
            .await
            .insert(config.name.clone(), channel);

        tracing::info!("Updated provider instance: {}", config.name);
        Ok(())
    }

    /// Delete a provider instance
    ///
    /// 1. Removes from database
    /// 2. Removes from in-memory registry (connection closed automatically)
    pub async fn delete(&self, name: &str) -> anyhow::Result<()> {
        // Remove from database
        self.repository.delete(name).await?;

        // Remove from memory (connection auto-closed)
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
        let channel = Self::create_grpc_channel(&config).await?;
        self.instances
            .write()
            .await
            .insert(config.name.clone(), channel);

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

        for (name, _channel) in instances.iter() {
            // For now, assume healthy if channel exists
            // TODO: Implement actual health check RPC
            let is_healthy = true;
            results.insert(name.clone(), is_healthy);
        }

        results
    }
}
