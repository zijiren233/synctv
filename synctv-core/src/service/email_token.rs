//! Email token service for email verification and password reset
//!
//! Manages generation, validation, and cleanup of email tokens.

use chrono::{Duration, Utc};
use nanoid::nanoid;
use sqlx::PgPool;
use tracing::{debug, info};

use crate::{
    models::UserId,
    repository::EmailTokenRepository,
    Error, Result,
};

/// Token type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailTokenType {
    EmailVerification,
    PasswordReset,
}

impl EmailTokenType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EmailTokenType::EmailVerification => "email_verification",
            EmailTokenType::PasswordReset => "password_reset",
        }
    }

    pub fn expiration_duration(&self) -> Duration {
        match self {
            EmailTokenType::EmailVerification => Duration::hours(24),  // 24 hours
            EmailTokenType::PasswordReset => Duration::hours(1),      // 1 hour
        }
    }
}

/// Email token service
#[derive(Clone)]
pub struct EmailTokenService {
    repository: EmailTokenRepository,
}

impl EmailTokenService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            repository: EmailTokenRepository::new(pool),
        }
    }

    /// Generate a new email token
    pub async fn generate_token(
        &self,
        user_id: &UserId,
        token_type: EmailTokenType,
    ) -> Result<String> {
        // Generate random token
        let token = nanoid!(64);

        let expires_at = Utc::now() + token_type.expiration_duration();

        self.repository
            .create(&token, user_id, token_type, expires_at)
            .await?;

        debug!(
            "Generated {} token for user {}",
            token_type.as_str(),
            user_id.as_str()
        );

        Ok(token)
    }

    /// Validate and consume an email token
    ///
    /// Returns the user_id if token is valid
    /// Marks token as used
    pub async fn validate_token(
        &self,
        token: &str,
        token_type: EmailTokenType,
    ) -> Result<UserId> {
        // Get token record
        let token_record = self
            .repository
            .get(token)
            .await?
            .ok_or_else(|| Error::InvalidInput("Invalid or expired token".to_string()))?;

        // Check token type
        if token_record.token_type != token_type.as_str() {
            return Err(Error::InvalidInput("Invalid token type".to_string()));
        }

        // Check if already used
        if token_record.used_at.is_some() {
            return Err(Error::InvalidInput("Token already used".to_string()));
        }

        // Check expiration
        if token_record.expires_at < Utc::now() {
            return Err(Error::InvalidInput("Token expired".to_string()));
        }

        // Mark as used
        self.repository.mark_as_used(token).await?;

        info!(
            "Validated {} token for user {}",
            token_type.as_str(),
            token_record.user_id.as_str()
        );

        Ok(token_record.user_id)
    }

    /// Invalidate all tokens of a specific type for a user
    pub async fn invalidate_user_tokens(
        &self,
        user_id: &UserId,
        token_type: EmailTokenType,
    ) -> Result<()> {
        self.repository
            .delete_user_tokens(user_id, token_type)
            .await?;

        debug!(
            "Invalidated all {} tokens for user {}",
            token_type.as_str(),
            user_id.as_str()
        );

        Ok(())
    }

    /// Cleanup expired tokens
    pub async fn cleanup_expired(&self) -> Result<usize> {
        let count = self.repository.cleanup_expired().await?;
        if count > 0 {
            info!("Cleaned up {} expired email tokens", count);
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_type_expiration() {
        let email_verify = EmailTokenType::EmailVerification;
        let password_reset = EmailTokenType::PasswordReset;

        assert_eq!(email_verify.as_str(), "email_verification");
        assert_eq!(password_reset.as_str(), "password_reset");

        // Email verification: 24 hours
        assert_eq!(email_verify.expiration_duration(), Duration::hours(24));

        // Password reset: 1 hour
        assert_eq!(password_reset.expiration_duration(), Duration::hours(1));
    }
}
