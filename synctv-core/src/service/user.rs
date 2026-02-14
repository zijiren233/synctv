use sqlx::PgPool;
use chrono::Utc;
use std::collections::HashMap;

use crate::{
    cache::UsernameCache,
    models::{User, UserId, SignupMethod},
    models::oauth2_client::OAuth2Provider,
    repository::UserRepository,
    service::auth::{hash_password, verify_password, JwtService, TokenType},
    service::TokenBlacklistService,
    Error, Result,
};

/// User service for business logic
#[derive(Clone)]
pub struct UserService {
    pub(crate) repository: UserRepository,
    jwt_service: JwtService,
    blacklist_service: TokenBlacklistService,
    username_cache: UsernameCache,
}

impl std::fmt::Debug for UserService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserService")
            .field("username_cache", &self.username_cache)
            .finish()
    }
}

impl UserService {
    #[must_use] 
    pub const fn new(
        pool: PgPool,
        jwt_service: JwtService,
        blacklist_service: TokenBlacklistService,
        username_cache: UsernameCache,
    ) -> Self {
        Self {
            repository: UserRepository::new(pool),
            jwt_service,
            blacklist_service,
            username_cache,
        }
    }

    /// Register a new user
    ///
    /// Uniqueness of username/email is enforced atomically by the database
    /// UNIQUE constraints, avoiding any check-then-act (TOCTOU) race condition.
    pub async fn register(
        &self,
        username: String,
        email: Option<String>,
        password: String,
    ) -> Result<(User, String, String)> {
        // Validate input
        self.validate_username(&username)?;
        if let Some(ref email) = email {
            self.validate_email(email)?;
        }
        self.validate_password(&password)?;

        // Hash password
        let password_hash = hash_password(&password).await?;

        // Create user with email signup method.
        // The database UNIQUE constraints on username and email will reject
        // duplicates atomically -- no separate existence check needed.
        let user = User::new(username.clone(), email.clone(), password_hash, Some(SignupMethod::Email));
        let created_user = self.repository.create(&user).await?;

        // Populate username cache
        self.username_cache.set(&created_user.id, &username).await?;

        // Generate JWT tokens (role will be fetched from DB on each request)
        let access_token = self
            .jwt_service
            .sign_token(&created_user.id, TokenType::Access)?;
        let refresh_token = self
            .jwt_service
            .sign_token(&created_user.id, TokenType::Refresh)?;

        Ok((created_user, access_token, refresh_token))
    }

    /// Login user
    ///
    /// Timing-safe: always performs password verification regardless of user existence
    /// to prevent username enumeration via response time analysis.
    pub async fn login(
        &self,
        username: String,
        password: String,
    ) -> Result<(User, String, String)> {
        // Get user by username
        let maybe_user = self
            .repository
            .get_by_username(&username)
            .await?;

        // Always perform password verification to prevent timing side-channel.
        // If the user doesn't exist, verify against a dummy hash so the response
        // time is indistinguishable from a real verification.
        let (is_valid, user) = match maybe_user {
            Some(user) => {
                let valid = verify_password(&password, &user.password_hash).await?;
                (valid, Some(user))
            }
            None => {
                // Dummy Argon2 verification to match timing of real verification.
                // This hash is pre-computed and never matches any real password.
                let dummy_hash = "$argon2id$v=19$m=65536,t=3,p=4$c29tZXNhbHQ$RdescudvJCsgt3ub+b+daw";
                let _ = verify_password(&password, dummy_hash).await;
                (false, None)
            }
        };

        // After constant-time verification, check all failure conditions
        let user = match user {
            Some(u) if is_valid => u,
            _ => return Err(Error::Authentication("Invalid username or password".to_string())),
        };

        // Check if user is banned or soft-deleted (generic message to prevent enumeration)
        if user.status == crate::models::UserStatus::Banned || user.deleted_at.is_some() {
            return Err(Error::Authentication("Invalid username or password".to_string()));
        }

        // Generate JWT tokens (role will be fetched from DB on each request)
        let access_token = self
            .jwt_service
            .sign_token(&user.id, TokenType::Access)?;
        let refresh_token = self
            .jwt_service
            .sign_token(&user.id, TokenType::Refresh)?;

        Ok((user, access_token, refresh_token))
    }

    /// Refresh access token
    pub async fn refresh_token(&self, refresh_token: String) -> Result<(String, String)> {
        // Verify refresh token
        let claims = self.jwt_service.verify_refresh_token(&refresh_token)?;

        // Get user to ensure they still exist and are active
        let user_id = UserId::from_string(claims.sub);
        let user = self
            .repository
            .get_by_id(&user_id)
            .await?
            .ok_or_else(|| Error::Authentication("User not found".to_string()))?;

        // Reject banned or soft-deleted users (generic message to prevent enumeration)
        if user.status == crate::models::UserStatus::Banned || user.deleted_at.is_some() {
            return Err(Error::Authentication("Authentication failed".to_string()));
        }

        // Reject refresh tokens issued before the last password change
        if self.blacklist_service.are_user_tokens_invalidated(&user_id, claims.iat).await? {
            return Err(Error::Authentication(
                "Token invalidated due to password change. Please log in again.".to_string(),
            ));
        }

        // Generate new tokens (role will be fetched from DB on each request)
        let new_access_token = self
            .jwt_service
            .sign_token(&user.id, TokenType::Access)?;
        let new_refresh_token = self
            .jwt_service
            .sign_token(&user.id, TokenType::Refresh)?;

        Ok((new_access_token, new_refresh_token))
    }

    /// Get user by ID
    pub async fn get_user(&self, user_id: &UserId) -> Result<User> {
        self.repository
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| Error::NotFound("User not found".to_string()))
    }

    /// Get user by email
    pub async fn get_by_email(&self, email: &str) -> Result<Option<User>> {
        self.repository.get_by_email(email).await
    }

    /// Update user (entire user object)
    pub async fn update_user(&self, user: &User) -> Result<User> {
        self.repository.update(user).await
    }

    /// Change user password (requires old password verification)
    pub async fn change_password(&self, user_id: &UserId, old_password: &str, new_password: &str) -> Result<User> {
        // Get user to verify old password
        let user = self.get_user(user_id).await?;

        // Verify old password
        let is_valid = verify_password(old_password, &user.password_hash).await?;
        if !is_valid {
            return Err(Error::Authentication("Invalid current password".to_string()));
        }

        // Delegate to set_password for the actual update
        self.set_password(user_id, new_password).await
    }

    /// Set user password (admin use, no old password required)
    ///
    /// After updating the password, all existing tokens for the user are
    /// invalidated so that stolen or leaked tokens cannot be reused.
    ///
    /// Token invalidation is performed **before** the database password
    /// update.  If Redis is unavailable the password change is aborted,
    /// because allowing the password to change while old tokens remain
    /// valid is a security risk.
    pub async fn set_password(&self, user_id: &UserId, new_password: &str) -> Result<User> {
        // Validate new password
        self.validate_password(new_password)?;

        // Hash new password
        let password_hash = hash_password(new_password).await?;

        // Invalidate all existing tokens for this user BEFORE updating the
        // password.  This ordering ensures that if Redis is down we abort
        // early and the password remains unchanged — a safe, consistent
        // state.  The reverse (password first, then invalidation) would
        // leave old tokens valid after a successful password change, which
        // is a security hole.
        //
        // TTL = 30 days (the maximum token lifetime, i.e. refresh tokens).
        // After 30 days, even old refresh tokens will have expired naturally.
        const THIRTY_DAYS_SECS: i64 = 30 * 24 * 60 * 60;
        if let Err(e) = self.blacklist_service.invalidate_user_tokens(user_id, THIRTY_DAYS_SECS).await {
            tracing::error!(
                user_id = %user_id.as_str(),
                error = %e,
                "SECURITY: Failed to invalidate tokens during password change — \
                 aborting password update to prevent old tokens from remaining valid"
            );
            return Err(Error::Internal(
                "Cannot securely process password change: \
                 token invalidation service is unavailable. \
                 Please try again later."
                    .to_string(),
            ));
        }

        // Update password in database (only reached if token invalidation succeeded)
        let updated_user = self.repository.update_password(user_id, &password_hash).await?;

        tracing::info!("Password updated for user {}", user_id.as_str());

        Ok(updated_user)
    }

    /// Set user email verification status
    pub async fn set_email_verified(&self, user_id: &UserId, email_verified: bool) -> Result<User> {
        let updated_user = self.repository.update_email_verified(user_id, email_verified).await?;

        tracing::info!(
            "Email verification status set to {} for user {}",
            email_verified,
            user_id.as_str()
        );

        Ok(updated_user)
    }

    /// List users with query (admin function)
    pub async fn list_users(&self, query: &crate::models::UserListQuery) -> Result<(Vec<User>, i64)> {
        self.repository.list(query).await
    }

    /// Logout user by blacklisting the access token
    pub async fn logout(&self, access_token: &str) -> Result<()> {
        // Decode token to get expiration time
        let claims = self.jwt_service.verify_access_token(access_token)?;

        // Calculate TTL (time until token expires)
        let now = Utc::now().timestamp();
        let ttl = claims.exp - now;

        if ttl > 0 {
            // Add token to blacklist with TTL
            self.blacklist_service.blacklist_token(access_token, ttl).await?;

            tracing::info!(
                user_id = %claims.sub,
                ttl_seconds = ttl,
                "User logged out, token blacklisted"
            );
        } else {
            tracing::debug!("Token already expired, no need to blacklist");
        }

        Ok(())
    }

    /// Check if a token is blacklisted
    pub async fn is_token_blacklisted(&self, token: &str) -> Result<bool> {
        self.blacklist_service.is_blacklisted(token).await
    }

    /// Check if a token has been invalidated by a password change.
    ///
    /// Returns `true` if the token was issued before the user's most recent
    /// password change and should therefore be rejected.
    pub async fn is_token_invalidated_by_password_change(
        &self,
        user_id: &UserId,
        token_iat: i64,
    ) -> Result<bool> {
        self.blacklist_service
            .are_user_tokens_invalidated(user_id, token_iat)
            .await
    }

    /// Create a new user for an `OAuth2` login.
    ///
    /// This method is called during `OAuth2` login flow when no existing provider
    /// mapping was found (the caller must check provider-based lookup first).
    /// It creates a new user with a random password.
    ///
    /// If the desired username is already taken (detected atomically via DB
    /// UNIQUE constraint), a numeric suffix is appended (e.g., "alice" ->
    /// "`alice_2`", "`alice_3`") to avoid collisions. This prevents account
    /// takeover where an `OAuth2` user with a matching username would silently
    /// gain access to an existing local account.
    ///
    /// Note: This method doesn't save the `OAuth2` provider mapping - that's handled
    /// by `OAuth2Service::upsert_user_provider`.
    /// Note: Email is optional for `OAuth2` users.
    pub async fn create_or_load_by_oauth2(
        &self,
        provider: &OAuth2Provider,
        provider_user_id: &str,
        username: &str,
        email: Option<&str>,
    ) -> Result<User> {
        // Generate a random password (OAuth2 users don't need password login)
        let random_password = nanoid::nanoid!(32);

        // Use provided email, or None if not provided
        let user_email = email.map(std::string::ToString::to_string);

        // Hash password
        let password_hash = hash_password(&random_password).await?;

        // Try to create user with the desired username first. If the DB UNIQUE
        // constraint rejects it, fall back to suffixed variants. This avoids
        // the TOCTOU race of check-then-insert.
        let candidates = std::iter::once(username.to_string())
            .chain((2..=1000).map(|suffix| {
                // Cap the base to leave room for the suffix within the 50-char limit
                let max_base_len = 45;
                let base = if username.len() > max_base_len {
                    &username[..max_base_len]
                } else {
                    username
                };
                format!("{base}_{suffix}")
            }));

        for candidate in candidates {
            let user = User::new(
                candidate.clone(),
                user_email.clone(),
                password_hash.clone(),
                Some(SignupMethod::OAuth2),
            );
            match self.repository.create(&user).await {
                Ok(created_user) => {
                    // Populate username cache
                    self.username_cache.set(&created_user.id, &candidate).await?;

                    if candidate == username {
                        tracing::info!(
                            "Created new user {} (username='{}') via OAuth2 provider {} (provider_user_id={})",
                            created_user.id.as_str(),
                            candidate,
                            provider.as_str(),
                            provider_user_id
                        );
                    } else {
                        tracing::info!(
                            "Username '{}' was taken; created user {} as '{}' via OAuth2 provider {} (provider_user_id={})",
                            username,
                            created_user.id.as_str(),
                            candidate,
                            provider.as_str(),
                            provider_user_id
                        );
                    }

                    return Ok(created_user);
                }
                Err(Error::AlreadyExists(ref msg)) if msg.contains("Username") => {
                    // Username conflict -- try next candidate
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        Err(Error::Internal(format!(
            "Could not generate a unique username for base '{username}' after 1000 attempts"
        )))
    }

    /// Validate username using production-grade validator
    fn validate_username(&self, username: &str) -> Result<()> {
        crate::validation::UsernameValidator::new()
            .validate(username)
            .map_err(|e| Error::InvalidInput(e.to_string()))
    }

    /// Validate email using regex-based validator
    fn validate_email(&self, email: &str) -> Result<()> {
        let email = email.trim();
        if email.is_empty() {
            return Err(Error::InvalidInput("Email cannot be empty".to_string()));
        }
        crate::validation::EmailValidator::new()
            .validate(email)
            .map_err(|e| Error::InvalidInput(e.to_string()))
    }

    /// Validate password with complexity requirements
    fn validate_password(&self, password: &str) -> Result<()> {
        crate::validation::PasswordValidator::new()
            .validate(password)
            .map_err(|e| Error::InvalidInput(e.to_string()))
    }

    /// Get username for a user ID (from cache or database)
    ///
    /// This method checks the cache first, then falls back to the database.
    /// The cache is automatically populated on cache miss.
    pub async fn get_username(&self, user_id: &UserId) -> Result<Option<String>> {
        // Check cache first
        if let Some(username) = self.username_cache.get(user_id).await? {
            return Ok(Some(username));
        }

        // Cache miss - fetch from database
        if let Some(user) = self.repository.get_by_id(user_id).await? {
            // Populate cache
            let username = user.username.clone();
            self.username_cache.set(user_id, &username).await?;
            Ok(Some(username))
        } else {
            Ok(None)
        }
    }

    /// Get multiple usernames at once (more efficient)
    ///
    /// Returns a map of `user_id` -> username.
    pub async fn get_usernames(&self, user_ids: &[UserId]) -> Result<HashMap<UserId, String>> {
        // Try batch cache lookup first
        let mut result = self.username_cache.get_batch(user_ids).await?;
        let missing_ids: Vec<UserId> = user_ids
            .iter()
            .filter(|id| !result.contains_key(*id))
            .cloned()
            .collect();

        // Fetch missing usernames from database in a single batch query
        if !missing_ids.is_empty() {
            let users = self.repository.get_by_ids(&missing_ids).await?;
            for user in users {
                let user_id = user.id.clone();
                let username = user.username.clone();
                self.username_cache.set(&user_id, &username).await?;
                result.insert(user_id, username);
            }
        }

        Ok(result)
    }

    /// Invalidate username cache for a user
    ///
    /// This should be called when a user's username is changed.
    pub async fn invalidate_username_cache(&self, user_id: &UserId) -> Result<()> {
        self.username_cache.invalidate(user_id).await
    }

    /// Get the database pool (for creating dependent services)
    #[must_use] 
    pub const fn pool(&self) -> &PgPool {
        self.repository.pool()
    }

    /// Get the username cache (for creating dependent services)
    #[must_use]
    pub const fn username_cache(&self) -> &UsernameCache {
        &self.username_cache
    }

    /// Health check - verify database connectivity
    ///
    /// Executes a simple query to verify the database connection is working.
    /// Used by readiness probes in Kubernetes deployments.
    ///
    /// # Returns
    /// - `Ok(())` if the database is accessible
    /// - `Err` if the database connection fails
    pub async fn health_check(&self) -> Result<()> {
        // Execute a simple query to verify database connectivity
        sqlx::query("SELECT 1")
            .execute(self.pool())
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to create a test service with dummy JWT secret
    fn create_test_service() -> UserService {
        let pool = PgPool::connect_lazy("postgresql://fake").unwrap();

        let jwt = JwtService::new("test-secret-for-user-service").unwrap();
        let blacklist = TokenBlacklistService::new(None); // Disabled for tests
        let username_cache = UsernameCache::new(None, "test:".to_string(), 10, 0);
        UserService::new(pool, jwt, blacklist, username_cache)
    }

    #[tokio::test]
    async fn test_validate_username() {
        let service = create_test_service();

        assert!(service.validate_username("abc").is_ok());
        assert!(service.validate_username("user123").is_ok());
        assert!(service.validate_username("user_name").is_ok());
        assert!(service.validate_username("user-name").is_ok());

        assert!(service.validate_username("ab").is_err()); // Too short
        assert!(service.validate_username(&"a".repeat(51)).is_err()); // Too long
        assert!(service.validate_username("user@name").is_err()); // Invalid char
    }

    #[tokio::test]
    async fn test_validate_password() {
        let service = create_test_service();

        assert!(service.validate_password("password123").is_ok());
        assert!(service.validate_password("Pass123!").is_ok());

        assert!(service.validate_password("short").is_err()); // Too short
        assert!(service.validate_password(&"a".repeat(129)).is_err()); // Too long
    }
}
