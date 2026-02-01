use sqlx::PgPool;
use chrono::Utc;

use crate::{
    models::{User, UserId},
    repository::UserRepository,
    service::auth::{hash_password, verify_password, JwtService, TokenType},
    service::TokenBlacklistService,
    Error, Result,
};

/// User service for business logic
#[derive(Clone)]
pub struct UserService {
    repository: UserRepository,
    jwt_service: JwtService,
    blacklist_service: TokenBlacklistService,
}

impl std::fmt::Debug for UserService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserService").finish()
    }
}

impl UserService {
    pub fn new(pool: PgPool, jwt_service: JwtService, blacklist_service: TokenBlacklistService) -> Self {
        Self {
            repository: UserRepository::new(pool),
            jwt_service,
            blacklist_service,
        }
    }

    /// Register a new user
    pub async fn register(
        &self,
        username: String,
        email: String,
        password: String,
    ) -> Result<(User, String, String)> {
        // Validate input
        self.validate_username(&username)?;
        self.validate_email(&email)?;
        self.validate_password(&password)?;

        // Check if username already exists
        if self.repository.username_exists(&username).await? {
            return Err(Error::InvalidInput("Username already exists".to_string()));
        }

        // Check if email already exists
        if self.repository.email_exists(&email).await? {
            return Err(Error::InvalidInput("Email already exists".to_string()));
        }

        // Hash password
        let password_hash = hash_password(&password).await?;

        // Create user
        let user = User::new(username, email, password_hash);
        let created_user = self.repository.create(&user).await?;

        // Generate JWT tokens
        let access_token = self
            .jwt_service
            .sign_token(&created_user.id, created_user.permissions.0, TokenType::Access)?;
        let refresh_token = self
            .jwt_service
            .sign_token(&created_user.id, created_user.permissions.0, TokenType::Refresh)?;

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
            .sign_token(&user.id, user.permissions.0, TokenType::Access)?;
        let refresh_token = self
            .jwt_service
            .sign_token(&user.id, user.permissions.0, TokenType::Refresh)?;

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
            .sign_token(&user.id, user.permissions.0, TokenType::Access)?;
        let new_refresh_token = self
            .jwt_service
            .sign_token(&user.id, user.permissions.0, TokenType::Refresh)?;

        Ok((new_access_token, new_refresh_token))
    }

    /// Get user by ID
    pub async fn get_user(&self, user_id: &UserId) -> Result<User> {
        self.repository
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| Error::NotFound("User not found".to_string()))
    }

    /// Update user
    pub async fn update_user(&self, user: &User) -> Result<User> {
        self.repository.update(user).await
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
        if !email.contains('@') {
            return Err(Error::InvalidInput("Invalid email address".to_string()));
        }
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
        UserService::new(pool, jwt, blacklist)
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
