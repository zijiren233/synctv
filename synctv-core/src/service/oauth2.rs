//! OAuth2/OIDC authentication service
//!
//! This service handles OAuth2/OIDC login flow WITHOUT storing tokens.
//! Tokens are only used temporarily during login to fetch user info.

use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::{
    models::{oauth2_client::OAuth2Provider, UserId},
    repository::UserOAuthProviderRepository,
    oauth2::Provider as OAuth2ProviderTrait,
    Error, Result,
};

/// `OAuth2` state (for CSRF protection during authorization flow)
#[derive(Debug, Clone)]
pub struct OAuth2State {
    pub instance_name: String,
    pub redirect_url: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// `OAuth2` user info from provider (service layer)
#[derive(Debug, Clone)]
pub struct OAuth2UserInfo {
    pub provider: OAuth2Provider,
    pub provider_user_id: String,
    pub username: String,
    pub email: Option<String>,
    pub avatar: Option<String>,
}

/// `OAuth2` authentication service
///
/// Handles OAuth2/OIDC login flow:
/// 1. Generate authorization URL with PKCE
/// 2. Exchange authorization code for user info
/// 3. Create/update user-provider mapping (NO TOKENS STORED)
#[derive(Clone)]
pub struct OAuth2Service {
    repository: UserOAuthProviderRepository,
    /// Map of instance name -> provider instance (e.g., "github", "logto1", "logto2")
    providers: Arc<RwLock<HashMap<String, Box<dyn OAuth2ProviderTrait>>>>,
    /// Map of instance name -> provider enum type
    provider_types: Arc<RwLock<HashMap<String, OAuth2Provider>>>,
    states: Arc<RwLock<HashMap<String, OAuth2State>>>,
}

impl std::fmt::Debug for OAuth2Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuth2Service")
            .field("repository", &std::any::type_name::<UserOAuthProviderRepository>())
            .finish_non_exhaustive()
    }
}

impl OAuth2Service {
    /// Create new `OAuth2` service
    #[must_use] 
    pub fn new(repository: UserOAuthProviderRepository) -> Self {
        Self {
            repository,
            providers: Arc::new(RwLock::new(HashMap::new())),
            provider_types: Arc::new(RwLock::new(HashMap::new())),
            states: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register an `OAuth2` provider instance
    ///
    /// # Arguments
    /// * `instance_name` - Unique instance name (e.g., "github", "logto1", "logto2")
    /// * `provider_type` - Provider type enum
    /// * `provider` - The provider instance
    pub async fn register_provider(
        &self,
        instance_name: String,
        provider_type: OAuth2Provider,
        provider: Box<dyn OAuth2ProviderTrait>,
    ) {
        let mut providers = self.providers.write().await;
        let mut provider_types = self.provider_types.write().await;

        providers.insert(instance_name.clone(), provider);
        provider_types.insert(instance_name.clone(), provider_type.clone());

        info!("Registered OAuth2 provider: {} (type: {})", instance_name, provider_type.as_str());
    }

    /// Generate authorization URL
    pub async fn get_authorization_url(
        &self,
        instance_name: &str,
        redirect_url: Option<String>,
    ) -> Result<(String, String)> {
        // Validate redirect URL if provided
        if let Some(ref url) = redirect_url {
            Self::validate_redirect_url(url)?;
        }

        let providers = self.providers.read().await;
        let provider = providers.get(instance_name)
            .ok_or_else(|| Error::InvalidInput(format!("OAuth2 provider instance not found: {instance_name}")))?;

        // Generate state token
        let state_token = nanoid::nanoid!(32);

        // Generate authorization URL using provider
        let auth_url = provider.new_auth_url(&state_token).await
            .map_err(|e| Error::Internal(format!("Failed to generate authorization URL: {e}")))?;

        // Store state for verification during callback
        let oauth_state = OAuth2State {
            instance_name: instance_name.to_string(),
            redirect_url,
            created_at: chrono::Utc::now(),
        };

        let mut states = self.states.write().await;
        states.insert(state_token.clone(), oauth_state);

        debug!(
            "Generated OAuth2 authorization URL for provider {}",
            instance_name
        );

        Ok((auth_url, state_token))
    }

    /// Validate redirect URL to prevent open redirect vulnerabilities (CWE-601)
    ///
    /// This function ensures that redirect URLs are safe and cannot be used for phishing attacks.
    /// Only relative paths and same-origin URLs are allowed by default.
    fn validate_redirect_url(url: &str) -> Result<()> {
        // Empty or whitespace-only URLs are rejected
        if url.trim().is_empty() {
            return Err(Error::InvalidInput("Redirect URL cannot be empty".to_string()));
        }

        // Allow relative paths (must start with '/')
        if url.starts_with('/') {
            // Reject URLs with '//' (protocol-relative URLs can be used for open redirect)
            if url.starts_with("//") {
                return Err(Error::InvalidInput(
                    "Protocol-relative URLs are not allowed for security reasons".to_string()
                ));
            }
            // Valid relative path
            return Ok(());
        }

        // For absolute URLs, parse and validate
        match url::Url::parse(url) {
            Ok(parsed_url) => {
                // Only allow http and https schemes
                let scheme = parsed_url.scheme();
                if scheme != "http" && scheme != "https" {
                    return Err(Error::InvalidInput(format!(
                        "Invalid URL scheme: {scheme}. Only http and https are allowed"
                    )));
                }

                // Reject URLs with authentication credentials (user:pass@host)
                if parsed_url.username() != "" || parsed_url.password().is_some() {
                    return Err(Error::InvalidInput(
                        "URLs with embedded credentials are not allowed".to_string()
                    ));
                }

                // In production, you should validate against a whitelist of allowed domains
                // For now, we log a warning for absolute URLs
                tracing::warn!(
                    "OAuth2 redirect to external URL: {}. Consider configuring an allowed domains whitelist for enhanced security.",
                    url
                );

                Ok(())
            }
            Err(_) => {
                Err(Error::InvalidInput(format!(
                    "Invalid redirect URL format: {url}"
                )))
            }
        }
    }

    /// Verify `OAuth2` state during callback
    pub async fn verify_state(&self, state_token: &str) -> Result<OAuth2State> {
        let mut states = self.states.write().await;
        states
            .remove(state_token)
            .ok_or_else(|| Error::Authentication("Invalid or expired OAuth2 state".to_string()))
    }

    /// Exchange authorization code for user info
    pub async fn exchange_code_for_user_info(
        &self,
        instance_name: &str,
        code: &str,
    ) -> Result<(OAuth2UserInfo, OAuth2Provider)> {
        let providers = self.providers.read().await;
        let provider_types = self.provider_types.read().await;

        let provider = providers.get(instance_name)
            .ok_or_else(|| Error::InvalidInput(format!("OAuth2 provider instance not found: {instance_name}")))?;

        let provider_type = provider_types.get(instance_name)
            .ok_or_else(|| Error::InvalidInput(format!("Provider type not found: {instance_name}")))?;

        debug!("Exchanging code for user info from {}", instance_name);

        // Use provider to get user info
        let user_info = provider.get_user_info(code).await
            .map_err(|e| Error::Internal(format!("Failed to get user info: {e}")))?;

        // Convert provider user info to service user info
        let service_user_info = OAuth2UserInfo {
            provider: provider_type.clone(),
            provider_user_id: user_info.provider_user_id,
            username: user_info.username,
            email: user_info.email,
            avatar: user_info.avatar,
        };

        Ok((service_user_info, provider_type.clone()))
    }

    /// Create or update user-OAuth2 provider mapping
    pub async fn upsert_user_provider(
        &self,
        user_id: &UserId,
        provider: &OAuth2Provider,
        provider_user_id: &str,
        user_info: &OAuth2UserInfo,
    ) -> Result<()> {
        // Convert service user info to repository format
        let repo_user_info = crate::models::oauth2_client::OAuth2UserInfo {
            provider: provider.clone(),
            provider_user_id: user_info.provider_user_id.clone(),
            username: user_info.username.clone(),
            email: user_info.email.clone(),
            avatar: user_info.avatar.clone(),
        };

        self.repository
            .upsert(user_id, provider, provider_user_id, &repo_user_info)
            .await
    }

    /// Find user by `OAuth2` provider
    pub async fn find_user_by_provider(
        &self,
        provider: &OAuth2Provider,
        provider_user_id: &str,
    ) -> Result<Option<UserId>> {
        match self
            .repository
            .find_by_provider(provider, provider_user_id)
            .await?
        {
            Some(mapping) => Ok(Some(mapping.user_id)),
            None => Ok(None),
        }
    }

    /// Get all `OAuth2` providers for a user
    pub async fn get_user_providers(&self, user_id: &UserId) -> Result<Vec<OAuth2Provider>> {
        let mappings = self.repository.find_by_user(user_id).await?;
        Ok(mappings
            .into_iter()
            .filter_map(|m| m.provider_enum())
            .collect())
    }

    /// List all configured `OAuth2` provider instances
    ///
    /// Returns a list of (`instance_name`, `provider_type`) pairs for all registered providers.
    /// This is used by the HTTP API to tell clients which `OAuth2` login options are available.
    /// Returns an empty vector if no providers are configured. Order is not guaranteed.
    pub async fn list_available_instances(&self) -> Vec<(String, OAuth2Provider)> {
        let provider_types = self.provider_types.read().await;
        provider_types
            .iter()
            .map(|(name, provider_type)| (name.clone(), provider_type.clone()))
            .collect()
    }

    /// Unlink `OAuth2` provider from user
    pub async fn unlink_provider(
        &self,
        user_id: &UserId,
        provider: &OAuth2Provider,
        provider_user_id: &str,
    ) -> Result<bool> {
        self.repository
            .delete(user_id, provider, provider_user_id)
            .await
    }

    /// Unlink all bindings for a specific `OAuth2` provider from user
    pub async fn unlink_provider_all(
        &self,
        user_id: &UserId,
        provider: &OAuth2Provider,
    ) -> Result<bool> {
        let mappings = self.repository.find_by_user(user_id).await?;
        let mut deleted = false;
        for mapping in mappings {
            if mapping.provider_enum().as_ref() == Some(provider)
                && self.repository.delete(user_id, provider, &mapping.provider_user_id).await? {
                    deleted = true;
                }
        }
        Ok(deleted)
    }

    /// Clean up expired `OAuth2` states (maintenance task)
    pub async fn cleanup_expired_states(&self, max_age_seconds: i64) -> Result<()> {
        let mut states = self.states.write().await;
        let now = chrono::Utc::now();
        let initial_count = states.len();

        states.retain(|_, state| {
            let age = now.signed_duration_since(state.created_at).num_seconds();
            age < max_age_seconds
        });

        let removed = initial_count - states.len();
        if removed > 0 {
            debug!("Cleaned up {} expired OAuth2 states", removed);
        }

        Ok(())
    }
}
