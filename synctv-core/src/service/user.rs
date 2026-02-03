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
    pub fn new(
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

        // Check if username already exists
        if self.repository.username_exists(&username).await? {
            return Err(Error::InvalidInput("Username already exists".to_string()));
        }

        // Check if email already exists (only if provided)
        if let Some(ref email) = email {
            if self.repository.email_exists(email).await? {
                return Err(Error::InvalidInput("Email already exists".to_string()));
            }
        }

        // Hash password
        let password_hash = hash_password(&password).await?;

        // Create user with email signup method
        let user = User::new(username.clone(), email.clone(), password_hash, Some(SignupMethod::Email));
        let created_user = self.repository.create(&user).await?;

        // Populate username cache
        self.username_cache.set(&created_user.id, &username).await?;

        // Generate JWT tokens
        let access_token = self
            .jwt_service
            .sign_token(&created_user.id, created_user.role, TokenType::Access)?;
        let refresh_token = self
            .jwt_service
            .sign_token(&created_user.id, created_user.role, TokenType::Refresh)?;

        Ok((created_user, access_token, refresh_token))
    }

    /// Login user
    pub async fn login(
        &self,
        username: String,
        password: String,
    ) -> Result<(User, String, String)> {
        // Get user by username
        let user = self
            .repository
            .get_by_username(&username)
            .await?
            .ok_or_else(|| Error::Authentication("Invalid username or password".to_string()))?;

        // Verify password
        let is_valid = verify_password(&password, &user.password_hash).await?;
        if !is_valid {
            return Err(Error::Authentication("Invalid username or password".to_string()));
        }

        // Generate JWT tokens
        let access_token = self
            .jwt_service
            .sign_token(&user.id, user.role, TokenType::Access)?;
        let refresh_token = self
            .jwt_service
            .sign_token(&user.id, user.role, TokenType::Refresh)?;

        Ok((user, access_token, refresh_token))
    }

    /// Refresh access token
    pub async fn refresh_token(&self, refresh_token: String) -> Result<(String, String)> {
        // Verify refresh token
        let claims = self.jwt_service.verify_refresh_token(&refresh_token)?;

        // Get user to ensure they still exist
        let user_id = UserId::from_string(claims.sub);
        let user = self
            .repository
            .get_by_id(&user_id)
            .await?
            .ok_or_else(|| Error::Authentication("User not found".to_string()))?;

        // Generate new tokens
        let new_access_token = self
            .jwt_service
            .sign_token(&user.id, user.role, TokenType::Access)?;
        let new_refresh_token = self
            .jwt_service
            .sign_token(&user.id, user.role, TokenType::Refresh)?;

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

    /// Set user password
    pub async fn set_password(&self, user_id: &UserId, new_password: &str) -> Result<User> {
        // Validate new password
        self.validate_password(new_password)?;

        // Hash new password
        let password_hash = hash_password(new_password).await?;

        // Update password in database
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

    /// Create or load user by OAuth2 provider
    ///
    /// This method is called during OAuth2 login flow.
    /// If a user exists with the given provider and provider_user_id, return it.
    /// Otherwise, create a new user with a random password.
    ///
    /// Note: This method doesn't save the OAuth2 token - that's handled by OAuth2Service.
    /// Note: Email is optional for OAuth2 users.
    pub async fn create_or_load_by_oauth2(
        &self,
        provider: &OAuth2Provider,
        _provider_user_id: &str,
        username: &str,
        email: Option<&str>,
    ) -> Result<User> {
        // Check if user already exists by username
        if let Some(user) = self.repository.get_by_username(username).await? {
            tracing::info!(
                "Found existing user {} by username during OAuth2 login",
                user.id.as_str()
            );
            return Ok(user);
        }

        // Generate a random password (OAuth2 users don't need password login)
        let random_password = nanoid::nanoid!(32);

        // Use provided email, or None if not provided
        let user_email = email.map(|e| e.to_string());

        // Hash password
        let password_hash = hash_password(&random_password).await?;

        // Create user with OAuth2 signup method
        let user = User::new(username.to_string(), user_email, password_hash, Some(SignupMethod::OAuth2));
        let created_user = self.repository.create(&user).await?;

        // Populate username cache
        self.username_cache.set(&created_user.id, username).await?;

        tracing::info!(
            "Created new user {} via OAuth2 provider {}",
            created_user.id.as_str(),
            provider.as_str()
        );

        Ok(created_user)
    }

    /// Validate username
    fn validate_username(&self, username: &str) -> Result<()> {
        if username.len() < 3 {
            return Err(Error::InvalidInput(
                "Username must be at least 3 characters".to_string(),
            ));
        }
        if username.len() > 50 {
            return Err(Error::InvalidInput(
                "Username must be at most 50 characters".to_string(),
            ));
        }
        if !username
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Err(Error::InvalidInput(
                "Username can only contain alphanumeric characters, underscores, and hyphens".to_string(),
            ));
        }
        Ok(())
    }

    /// Validate email
    fn validate_email(&self, email: &str) -> Result<()> {
        let email = email.trim();

        // Check if empty after trim
        if email.is_empty() {
            return Err(Error::InvalidInput("Email cannot be empty or whitespace only".to_string()));
        }

        // Check for @ symbol
        if !email.contains('@') {
            return Err(Error::InvalidInput("Invalid email address".to_string()));
        }

        // Check length
        if email.len() > 255 {
            return Err(Error::InvalidInput(
                "Email must be at most 255 characters".to_string(),
            ));
        }

        Ok(())
    }

    /// Validate password
    fn validate_password(&self, password: &str) -> Result<()> {
        if password.len() < 8 {
            return Err(Error::InvalidInput(
                "Password must be at least 8 characters".to_string(),
            ));
        }
        if password.len() > 128 {
            return Err(Error::InvalidInput(
                "Password must be at most 128 characters".to_string(),
            ));
        }
        Ok(())
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
    /// Returns a map of user_id -> username.
    pub async fn get_usernames(&self, user_ids: &[UserId]) -> Result<HashMap<UserId, String>> {
        // Try batch cache lookup first
        let mut result = self.username_cache.get_batch(user_ids).await?;
        let missing_ids: Vec<UserId> = user_ids
            .iter()
            .filter(|id| !result.contains_key(*id))
            .cloned()
            .collect();

        // Fetch missing usernames from database
        if !missing_ids.is_empty() {
            for user_id in &missing_ids {
                if let Some(user) = self.repository.get_by_id(user_id).await? {
                    let username = user.username.clone();
                    self.username_cache.set(user_id, &username).await?;
                    result.insert(user_id.clone(), username);
                }
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
    pub fn pool(&self) -> &PgPool {
        self.repository.pool()
    }

    /// Get the username cache (for creating dependent services)
    pub fn username_cache(&self) -> &UsernameCache {
        &self.username_cache
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to create a test service with dummy keys
    fn create_test_service() -> UserService {
        let pool = PgPool::connect_lazy("postgresql://fake").unwrap();

        // Generate test RSA keys
        use rsa::RsaPrivateKey;
        use rand::rngs::OsRng;
        let mut rng = OsRng;
        let private_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();

        // Encode to PEM
        use rsa::pkcs8::EncodePrivateKey;
        let private_pem = private_key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap();
        let private_bytes = private_pem.as_bytes();

        use rsa::RsaPublicKey;
        use rsa::pkcs8::EncodePublicKey;
        let public_key = RsaPublicKey::from(&private_key);
        let public_pem = public_key.to_public_key_pem(rsa::pkcs8::LineEnding::LF).unwrap();
        let public_bytes = public_pem.as_bytes();

        let jwt = JwtService::new(private_bytes, public_bytes).unwrap();
        let blacklist = TokenBlacklistService::new(None).unwrap(); // Disabled for tests
        let username_cache = UsernameCache::new(None, "test:".to_string(), 10, 0).unwrap();
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
