use redis::AsyncCommands;
use sha2::{Sha256, Digest};
use std::sync::Arc;
use std::time::Duration;
use crate::{models::UserId, Error, Result};

/// Hash a token for use in Redis keys and log messages.
/// This prevents raw tokens from appearing in Redis key space or log aggregation systems.
fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Token blacklist service for managing revoked JWT tokens
///
/// Uses Redis when available for distributed blacklist. Falls back to
/// `moka` in-memory cache for per-instance blacklisting when Redis is
/// not configured.
#[derive(Clone)]
pub struct TokenBlacklistService {
    redis_conn: Option<redis::aio::ConnectionManager>,
    /// In-memory token blacklist: token_hash -> expiry_timestamp_secs
    local_blacklist: Arc<moka::future::Cache<String, i64>>,
    /// In-memory user invalidation timestamps: user_key -> password_changed_at
    local_user_invalidations: Arc<moka::future::Cache<String, i64>>,
}

impl TokenBlacklistService {
    /// Create a new `TokenBlacklistService`
    ///
    /// If `redis_conn` is None, falls back to per-instance in-memory blacklist
    /// using moka cache with TTL per entry.
    pub fn new(redis_conn: Option<redis::aio::ConnectionManager>) -> Self {
        if redis_conn.is_none() {
            tracing::warn!(
                "Token blacklist using in-memory fallback: Redis not configured. \
                 Revocations are per-instance only (not shared across replicas)."
            );
        }
        Self {
            redis_conn,
            // Max 100K blacklisted tokens in memory; 30-day max TTL covers refresh tokens
            local_blacklist: Arc::new(
                moka::future::Cache::builder()
                    .max_capacity(100_000)
                    .time_to_live(Duration::from_secs(30 * 24 * 3600))
                    .build(),
            ),
            // Max 50K user invalidation entries; 30-day max TTL
            local_user_invalidations: Arc::new(
                moka::future::Cache::builder()
                    .max_capacity(50_000)
                    .time_to_live(Duration::from_secs(30 * 24 * 3600))
                    .build(),
            ),
        }
    }

    /// Add a token to the blacklist
    /// The token will be blacklisted until it expires (`ttl_seconds`)
    pub async fn blacklist_token(&self, token: &str, ttl_seconds: i64) -> Result<()> {
        if ttl_seconds <= 0 {
            // Token already expired, no need to blacklist
            return Ok(());
        }

        let token_hash = hash_token(token);

        if let Some(ref conn) = self.redis_conn {
            let mut conn = conn.clone();

            let key = format!("token:blacklist:{token_hash}");

            let _: () = conn.set_ex(&key, "1", ttl_seconds as u64)
                .await
                .map_err(|e| Error::Internal(format!("Failed to blacklist token: {e}")))?;

            tracing::info!(token_hash = %&token_hash[..16], ttl_seconds, "Token blacklisted");
        } else {
            // In-memory fallback: store expiry timestamp
            let expires_at = chrono::Utc::now().timestamp() + ttl_seconds;
            self.local_blacklist.insert(token_hash.clone(), expires_at).await;
            tracing::info!(token_hash = %&token_hash[..16], ttl_seconds, "Token blacklisted (in-memory)");
        }

        Ok(())
    }

    /// Check if a token is blacklisted
    pub async fn is_blacklisted(&self, token: &str) -> Result<bool> {
        let token_hash = hash_token(token);

        if let Some(ref conn) = self.redis_conn {
            let mut conn = conn.clone();

            let key = format!("token:blacklist:{token_hash}");

            let exists: bool = conn.exists(&key)
                .await
                .map_err(|e| Error::Internal(format!("Failed to check token blacklist: {e}")))?;

            Ok(exists)
        } else {
            // In-memory fallback: check if entry exists and hasn't expired
            if let Some(expires_at) = self.local_blacklist.get(&token_hash).await {
                let now = chrono::Utc::now().timestamp();
                if now < expires_at {
                    return Ok(true);
                }
                // Expired - remove lazily
                self.local_blacklist.invalidate(&token_hash).await;
            }
            Ok(false)
        }
    }

    /// Remove a token from the blacklist (rarely needed, for testing)
    pub async fn remove_token(&self, token: &str) -> Result<()> {
        let token_hash = hash_token(token);

        if let Some(ref conn) = self.redis_conn {
            let mut conn = conn.clone();

            let key = format!("token:blacklist:{token_hash}");

            let _: () = conn.del(&key)
                .await
                .map_err(|e| Error::Internal(format!("Failed to remove token from blacklist: {e}")))?;
        } else {
            self.local_blacklist.invalidate(&token_hash).await;
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

        let now = chrono::Utc::now().timestamp();

        if let Some(ref conn) = self.redis_conn {
            let mut conn = conn.clone();

            let key = format!("user:password_changed:{}", user_id.as_str());

            let _: () = conn.set_ex(&key, now.to_string(), ttl_seconds as u64)
                .await
                .map_err(|e| Error::Internal(format!("Failed to set user token invalidation: {e}")))?;

            tracing::info!(
                user_id = %user_id.as_str(),
                "All existing tokens invalidated for user (password changed)"
            );
        } else {
            // In-memory fallback
            let key = format!("user:password_changed:{}", user_id.as_str());
            self.local_user_invalidations.insert(key, now).await;
            tracing::info!(
                user_id = %user_id.as_str(),
                "All existing tokens invalidated for user (in-memory)"
            );
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
            // In-memory fallback
            let key = format!("user:password_changed:{}", user_id.as_str());
            if let Some(password_changed_at) = self.local_user_invalidations.get(&key).await {
                return Ok(token_iat <= password_changed_at);
            }
            Ok(false)
        }
    }

    /// Check if the service uses Redis (distributed mode)
    #[must_use]
    pub const fn uses_redis(&self) -> bool {
        self.redis_conn.is_some()
    }

    /// Check if the service is enabled (always true - uses in-memory fallback when no Redis)
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        true
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
    async fn test_in_memory_blacklist() {
        let service = TokenBlacklistService::new(None);

        assert!(service.is_enabled());
        assert!(!service.uses_redis());

        let token = "test_token_12345";

        // Initially not blacklisted
        assert!(!service.is_blacklisted(token).await.unwrap());

        // Blacklist it
        service.blacklist_token(token, 60).await.unwrap();

        // Now it should be blacklisted (in-memory)
        assert!(service.is_blacklisted(token).await.unwrap());

        // Remove it
        service.remove_token(token).await.unwrap();

        // Should not be blacklisted anymore
        assert!(!service.is_blacklisted(token).await.unwrap());
    }

    #[tokio::test]
    async fn test_in_memory_user_token_invalidation() {
        let service = TokenBlacklistService::new(None);
        let user_id = UserId::from_string("test_user".to_string());

        let now = chrono::Utc::now().timestamp();

        // Initially no invalidation
        assert!(!service.are_user_tokens_invalidated(&user_id, now).await.unwrap());

        // Invalidate tokens
        service.invalidate_user_tokens(&user_id, 3600).await.unwrap();

        // Token issued before invalidation should be rejected
        let old_iat = now - 10;
        assert!(service.are_user_tokens_invalidated(&user_id, old_iat).await.unwrap());

        // Token issued after invalidation should be accepted
        let future_iat = now + 10;
        assert!(!service.are_user_tokens_invalidated(&user_id, future_iat).await.unwrap());
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
