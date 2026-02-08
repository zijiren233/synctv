// Provider Instance Repository
//
// Database access layer for provider instance configuration management.

use crate::models::{ProviderInstance, UserProviderCredential};
use sqlx::{PgPool, Result};

/// Provider Instance Repository
pub struct ProviderInstanceRepository {
    pool: PgPool,
}

impl std::fmt::Debug for ProviderInstanceRepository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderInstanceRepository")
            .field("pool", &"PgPool")
            .finish()
    }
}

impl ProviderInstanceRepository {
    #[must_use] 
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get all provider instances
    pub async fn get_all(&self) -> Result<Vec<ProviderInstance>> {
        sqlx::query_as::<_, ProviderInstance>(
            "SELECT * FROM media_provider_instances ORDER BY created_at DESC"
        )
        .fetch_all(&self.pool)
        .await
    }

    /// Get all enabled provider instances
    pub async fn get_all_enabled(&self) -> Result<Vec<ProviderInstance>> {
        sqlx::query_as::<_, ProviderInstance>(
            "SELECT * FROM media_provider_instances WHERE enabled = true ORDER BY created_at DESC"
        )
        .fetch_all(&self.pool)
        .await
    }

    /// Get provider instance by name
    pub async fn get_by_name(&self, name: &str) -> Result<Option<ProviderInstance>> {
        sqlx::query_as::<_, ProviderInstance>(
            "SELECT * FROM media_provider_instances WHERE name = $1"
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
    }

    /// Get instances that support a specific provider type
    pub async fn find_by_provider(&self, provider: &str) -> Result<Vec<ProviderInstance>> {
        sqlx::query_as::<_, ProviderInstance>(
            "SELECT * FROM media_provider_instances WHERE $1 = ANY(providers) AND enabled = true"
        )
        .bind(provider)
        .fetch_all(&self.pool)
        .await
    }

    /// Create a new provider instance
    pub async fn create(&self, instance: &ProviderInstance) -> Result<()> {
        sqlx::query(
            r"
            INSERT INTO media_provider_instances
            (name, endpoint, comment, jwt_secret, custom_ca, timeout, tls, insecure_tls, providers, enabled)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "
        )
        .bind(&instance.name)
        .bind(&instance.endpoint)
        .bind(&instance.comment)
        .bind(&instance.jwt_secret)
        .bind(&instance.custom_ca)
        .bind(&instance.timeout)
        .bind(instance.tls)
        .bind(instance.insecure_tls)
        .bind(&instance.providers)
        .bind(instance.enabled)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Update an existing provider instance
    pub async fn update(&self, instance: &ProviderInstance) -> Result<()> {
        sqlx::query(
            r"
            UPDATE media_provider_instances
            SET endpoint = $2, comment = $3, jwt_secret = $4, custom_ca = $5,
                timeout = $6, tls = $7, insecure_tls = $8, providers = $9, enabled = $10,
                updated_at = NOW()
            WHERE name = $1
            "
        )
        .bind(&instance.name)
        .bind(&instance.endpoint)
        .bind(&instance.comment)
        .bind(&instance.jwt_secret)
        .bind(&instance.custom_ca)
        .bind(&instance.timeout)
        .bind(instance.tls)
        .bind(instance.insecure_tls)
        .bind(&instance.providers)
        .bind(instance.enabled)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Delete a provider instance
    pub async fn delete(&self, name: &str) -> Result<()> {
        sqlx::query("DELETE FROM media_provider_instances WHERE name = $1")
            .bind(name)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Enable a provider instance
    pub async fn enable(&self, name: &str) -> Result<()> {
        sqlx::query("UPDATE media_provider_instances SET enabled = true, updated_at = NOW() WHERE name = $1")
            .bind(name)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Disable a provider instance
    pub async fn disable(&self, name: &str) -> Result<()> {
        sqlx::query("UPDATE media_provider_instances SET enabled = false, updated_at = NOW() WHERE name = $1")
            .bind(name)
            .execute(&self.pool)
            .await?;

        Ok(())
    }
}

/// User Provider Credential Repository
pub struct UserProviderCredentialRepository {
    pool: PgPool,
}

impl std::fmt::Debug for UserProviderCredentialRepository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserProviderCredentialRepository")
            .field("pool", &"PgPool")
            .finish()
    }
}

impl UserProviderCredentialRepository {
    #[must_use] 
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get all credentials for a user
    pub async fn get_by_user(&self, user_id: &str) -> Result<Vec<UserProviderCredential>> {
        sqlx::query_as::<_, UserProviderCredential>(
            "SELECT * FROM user_media_provider_credentials WHERE user_id = $1 ORDER BY created_at DESC"
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
    }

    /// Get credential by ID
    pub async fn get_by_id(&self, id: &str) -> Result<Option<UserProviderCredential>> {
        sqlx::query_as::<_, UserProviderCredential>(
            "SELECT * FROM user_media_provider_credentials WHERE id = $1"
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
    }

    /// Get user credential for a specific provider and server
    pub async fn get_by_provider_and_server(
        &self,
        user_id: &str,
        provider: &str,
        server_id: &str,
    ) -> Result<Option<UserProviderCredential>> {
        sqlx::query_as::<_, UserProviderCredential>(
            "SELECT * FROM user_media_provider_credentials WHERE user_id = $1 AND provider = $2 AND server_id = $3"
        )
        .bind(user_id)
        .bind(provider)
        .bind(server_id)
        .fetch_optional(&self.pool)
        .await
    }

    /// Get all credentials for a specific provider type
    pub async fn get_by_provider(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Vec<UserProviderCredential>> {
        sqlx::query_as::<_, UserProviderCredential>(
            "SELECT * FROM user_media_provider_credentials WHERE user_id = $1 AND provider = $2"
        )
        .bind(user_id)
        .bind(provider)
        .fetch_all(&self.pool)
        .await
    }

    /// Create a new user credential
    pub async fn create(&self, credential: &UserProviderCredential) -> Result<()> {
        sqlx::query(
            r"
            INSERT INTO user_media_provider_credentials
            (id, user_id, provider, server_id, provider_instance_name, credential_data, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "
        )
        .bind(&credential.id)
        .bind(&credential.user_id)
        .bind(&credential.provider)
        .bind(&credential.server_id)
        .bind(&credential.provider_instance_name)
        .bind(&credential.credential_data)
        .bind(credential.expires_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Update an existing user credential
    pub async fn update(&self, credential: &UserProviderCredential) -> Result<()> {
        sqlx::query(
            r"
            UPDATE user_media_provider_credentials
            SET provider_instance_name = $2, credential_data = $3, expires_at = $4, updated_at = NOW()
            WHERE id = $1
            "
        )
        .bind(&credential.id)
        .bind(&credential.provider_instance_name)
        .bind(&credential.credential_data)
        .bind(credential.expires_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Delete a user credential
    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM user_media_provider_credentials WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Delete all credentials for a user and provider
    pub async fn delete_by_user_and_provider(&self, user_id: &str, provider: &str) -> Result<()> {
        sqlx::query("DELETE FROM user_media_provider_credentials WHERE user_id = $1 AND provider = $2")
            .bind(user_id)
            .bind(provider)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Get all expired credentials (for cleanup jobs)
    pub async fn get_expired(&self) -> Result<Vec<UserProviderCredential>> {
        sqlx::query_as::<_, UserProviderCredential>(
            "SELECT * FROM user_media_provider_credentials WHERE expires_at IS NOT NULL AND expires_at <= NOW()"
        )
        .fetch_all(&self.pool)
        .await
    }

    /// Delete all expired credentials
    pub async fn delete_expired(&self) -> Result<u64> {
        let result = sqlx::query(
            "DELETE FROM user_media_provider_credentials WHERE expires_at IS NOT NULL AND expires_at <= NOW()"
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These are unit tests for the repository structure.
    // Integration tests with actual database should be in tests/ directory.

    #[tokio::test]
    async fn test_repository_creation() {
        // This test just ensures the types compile correctly
        let pool = PgPool::connect_lazy("postgresql://test").unwrap();
        let _instance_repo = ProviderInstanceRepository::new(pool.clone());
        let _credential_repo = UserProviderCredentialRepository::new(pool);
    }
}
