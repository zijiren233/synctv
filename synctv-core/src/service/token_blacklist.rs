use redis::{AsyncCommands, Client};
use crate::{Error, Result};

/// Token blacklist service for managing revoked JWT tokens
#[derive(Clone)]
pub struct TokenBlacklistService {
    redis_client: Option<Client>,
}

impl TokenBlacklistService {
    /// Create a new `TokenBlacklistService`
    /// If `redis_url` is None, token blacklist will be disabled (logout becomes no-op)
    pub fn new(redis_url: Option<String>) -> Result<Self> {
        let redis_client = if let Some(url) = redis_url {
            Some(Client::open(url).map_err(|e| Error::Internal(format!("Failed to connect to Redis: {e}")))?)
        } else {
            None
        };

        Ok(Self { redis_client })
    }

    /// Add a token to the blacklist
    /// The token will be blacklisted until it expires (`ttl_seconds`)
    pub async fn blacklist_token(&self, token: &str, ttl_seconds: i64) -> Result<()> {
        if ttl_seconds <= 0 {
            // Token already expired, no need to blacklist
            return Ok(());
        }

        if let Some(client) = &self.redis_client {
            let mut conn = client.get_multiplexed_async_connection()
                .await
                .map_err(|e| Error::Internal(format!("Redis connection failed: {e}")))?;

            let key = format!("token:blacklist:{token}");

            let _: () = conn.set_ex(&key, "1", ttl_seconds as u64)
                .await
                .map_err(|e| Error::Internal(format!("Failed to blacklist token: {e}")))?;

            tracing::info!("Token blacklisted: {} (TTL: {}s)", &token[..10.min(token.len())], ttl_seconds);
        } else {
            tracing::warn!("Token blacklist disabled (Redis not configured)");
        }

        Ok(())
    }

    /// Check if a token is blacklisted
    pub async fn is_blacklisted(&self, token: &str) -> Result<bool> {
        if let Some(client) = &self.redis_client {
            let mut conn = client.get_multiplexed_async_connection()
                .await
                .map_err(|e| Error::Internal(format!("Redis connection failed: {e}")))?;

            let key = format!("token:blacklist:{token}");

            let exists: bool = conn.exists(&key)
                .await
                .map_err(|e| Error::Internal(format!("Failed to check token blacklist: {e}")))?;

            Ok(exists)
        } else {
            // If Redis is not configured, no tokens are blacklisted
            Ok(false)
        }
    }

    /// Remove a token from the blacklist (rarely needed, for testing)
    pub async fn remove_token(&self, token: &str) -> Result<()> {
        if let Some(client) = &self.redis_client {
            let mut conn = client.get_multiplexed_async_connection()
                .await
                .map_err(|e| Error::Internal(format!("Redis connection failed: {e}")))?;

            let key = format!("token:blacklist:{token}");

            let _: () = conn.del(&key)
                .await
                .map_err(|e| Error::Internal(format!("Failed to remove token from blacklist: {e}")))?;
        }

        Ok(())
    }

    /// Check if the service is enabled (Redis configured)
    #[must_use] 
    pub const fn is_enabled(&self) -> bool {
        self.redis_client.is_some()
    }
}

impl std::fmt::Debug for TokenBlacklistService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenBlacklistService")
            .field("enabled", &self.is_enabled())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Requires Redis"]
    async fn test_blacklist_token() {
        let service = TokenBlacklistService::new(Some("redis://localhost:6379".to_string())).unwrap();

        let token = "test_token_12345";

        // Initially not blacklisted
        assert!(!service.is_blacklisted(token).await.unwrap());

        // Blacklist it
        service.blacklist_token(token, 60).await.unwrap();

        // Now it should be blacklisted
        assert!(service.is_blacklisted(token).await.unwrap());

        // Remove it
        service.remove_token(token).await.unwrap();

        // Should not be blacklisted anymore
        assert!(!service.is_blacklisted(token).await.unwrap());
    }

    #[tokio::test]
    async fn test_disabled_blacklist() {
        let service = TokenBlacklistService::new(None).unwrap();

        assert!(!service.is_enabled());

        let token = "test_token_12345";

        // Should always return false when disabled
        assert!(!service.is_blacklisted(token).await.unwrap());

        // Blacklist should be no-op
        service.blacklist_token(token, 60).await.unwrap();
        assert!(!service.is_blacklisted(token).await.unwrap());
    }
}
