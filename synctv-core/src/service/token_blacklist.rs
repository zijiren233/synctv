use redis::AsyncCommands;
use crate::{models::UserId, Error, Result};

/// Token blacklist service for managing revoked JWT tokens
#[derive(Clone)]
pub struct TokenBlacklistService {
    redis_conn: Option<redis::aio::ConnectionManager>,
}

impl TokenBlacklistService {
    /// Create a new `TokenBlacklistService`
    /// If `redis_conn` is None, token blacklist will be disabled (logout becomes no-op)
    pub fn new(redis_conn: Option<redis::aio::ConnectionManager>) -> Self {
        if redis_conn.is_none() {
            tracing::warn!(
                "Token blacklist is disabled: Redis not configured. Tokens cannot be revoked. This is NOT safe for production."
            );
        }
        Self { redis_conn }
    }

    /// Add a token to the blacklist
    /// The token will be blacklisted until it expires (`ttl_seconds`)
    pub async fn blacklist_token(&self, token: &str, ttl_seconds: i64) -> Result<()> {
        if ttl_seconds <= 0 {
            // Token already expired, no need to blacklist
            return Ok(());
        }

        if let Some(ref conn) = self.redis_conn {
            let mut conn = conn.clone();

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
        if let Some(ref conn) = self.redis_conn {
            let mut conn = conn.clone();

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
        if let Some(ref conn) = self.redis_conn {
            let mut conn = conn.clone();

            let key = format!("token:blacklist:{token}");

            let _: () = conn.del(&key)
                .await
                .map_err(|e| Error::Internal(format!("Failed to remove token from blacklist: {e}")))?;
        }

        Ok(())
    }

    /// Invalidate all tokens for a user by storing the current timestamp.
    ///
    /// Any token with an `iat` (issued-at) before this timestamp will be
    /// rejected. The key is set with a TTL so it auto-expires once the
    /// longest-lived token (refresh token, 30 days) would have expired
    /// naturally.
    ///
    /// # Arguments
    /// * `user_id` - The user whose tokens should be invalidated
    /// * `ttl_seconds` - How long to keep the invalidation marker (should be
    ///   at least as long as the longest token lifetime, e.g. 30 days for
    ///   refresh tokens)
    pub async fn invalidate_user_tokens(&self, user_id: &UserId, ttl_seconds: i64) -> Result<()> {
        if ttl_seconds <= 0 {
            return Ok(());
        }

        if let Some(ref conn) = self.redis_conn {
            let mut conn = conn.clone();

            let key = format!("user:password_changed:{}", user_id.as_str());
            let now = chrono::Utc::now().timestamp();

            let _: () = conn.set_ex(&key, now.to_string(), ttl_seconds as u64)
                .await
                .map_err(|e| Error::Internal(format!("Failed to set user token invalidation: {e}")))?;

            tracing::info!(
                user_id = %user_id.as_str(),
                "All existing tokens invalidated for user (password changed)"
            );
        } else {
            tracing::warn!("Token invalidation skipped (Redis not configured)");
        }

        Ok(())
    }

    /// Check whether a token issued at `token_iat` has been invalidated by a
    /// password change for the given user.
    ///
    /// Returns `true` if the token should be rejected (i.e. it was issued
    /// before the most recent password change).
    pub async fn are_user_tokens_invalidated(&self, user_id: &UserId, token_iat: i64) -> Result<bool> {
        if let Some(ref conn) = self.redis_conn {
            let mut conn = conn.clone();

            let key = format!("user:password_changed:{}", user_id.as_str());

            let value: Option<String> = conn.get(&key)
                .await
                .map_err(|e| Error::Internal(format!("Failed to check user token invalidation: {e}")))?;

            if let Some(timestamp_str) = value {
                if let Ok(password_changed_at) = timestamp_str.parse::<i64>() {
                    // Reject tokens issued before the password change.
                    // We use <= to handle the edge case where a token was
                    // issued in the same second as the password change.
                    return Ok(token_iat <= password_changed_at);
                }
            }

            Ok(false)
        } else {
            // If Redis is not configured, no tokens are invalidated
            Ok(false)
        }
    }

    /// Check if the service is enabled (Redis configured)
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.redis_conn.is_some()
    }
}

impl std::fmt::Debug for TokenBlacklistService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenBlacklistService")
            .field("enabled", &self.redis_conn.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Requires Redis"]
    async fn test_blacklist_token() {
        let client = redis::Client::open("redis://localhost:6379").unwrap();
        let conn = redis::aio::ConnectionManager::new(client).await.unwrap();
        let service = TokenBlacklistService::new(Some(conn));

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
        let service = TokenBlacklistService::new(None);

        assert!(!service.is_enabled());

        let token = "test_token_12345";

        // Should always return false when disabled
        assert!(!service.is_blacklisted(token).await.unwrap());

        // Blacklist should be no-op
        service.blacklist_token(token, 60).await.unwrap();
        assert!(!service.is_blacklisted(token).await.unwrap());
    }

    #[tokio::test]
    async fn test_disabled_user_token_invalidation() {
        let service = TokenBlacklistService::new(None);
        let user_id = UserId::from_string("test_user".to_string());

        // Invalidation should be no-op when disabled
        service.invalidate_user_tokens(&user_id, 3600).await.unwrap();

        // Should always return false when disabled
        assert!(!service.are_user_tokens_invalidated(&user_id, 1000).await.unwrap());
    }

    #[tokio::test]
    #[ignore = "Requires Redis"]
    async fn test_user_token_invalidation() {
        let client = redis::Client::open("redis://localhost:6379").unwrap();
        let conn = redis::aio::ConnectionManager::new(client).await.unwrap();
        let service = TokenBlacklistService::new(Some(conn));
        let user_id = UserId::from_string("test_invalidation_user".to_string());

        // Initially no invalidation
        let now = chrono::Utc::now().timestamp();
        assert!(!service.are_user_tokens_invalidated(&user_id, now).await.unwrap());

        // Invalidate tokens
        service.invalidate_user_tokens(&user_id, 60).await.unwrap();

        // Token issued before invalidation should be rejected
        let old_iat = now - 10;
        assert!(service.are_user_tokens_invalidated(&user_id, old_iat).await.unwrap());

        // Token issued at the same second should also be rejected (edge case)
        assert!(service.are_user_tokens_invalidated(&user_id, now).await.unwrap());

        // Token issued after invalidation should be accepted
        let future_iat = now + 10;
        assert!(!service.are_user_tokens_invalidated(&user_id, future_iat).await.unwrap());
    }
}
