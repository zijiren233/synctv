//! Email verification and sending service
//!
//! Handles email verification and password reset email sending.

use chrono::{Duration, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{header::ContentType, Mailbox, MultiPart},
    transport::smtp::authentication::Credentials,
};

use crate::{Error, Result};
use super::email_token::{EmailTokenService, EmailTokenType};
use super::email_templates::EmailTemplateManager;

/// Email verification error
#[derive(Debug, thiserror::Error)]
pub enum EmailError {
    #[error("Email service not configured")]
    NotConfigured,

    #[error("Invalid email address: {0}")]
    InvalidEmail(String),

    #[error("Verification code expired")]
    CodeExpired,

    #[error("Invalid verification code")]
    InvalidCode,

    #[error("Too many attempts")]
    TooManyAttempts,

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("Send error: {0}")]
    SendError(String),
}

/// Verification code data
#[derive(Debug, Clone)]
struct VerificationCode {
    code: String,
    created_at: chrono::DateTime<Utc>,
    attempts: u32,
}

/// Email configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailConfig {
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_username: String,
    pub smtp_password: String,
    pub from_email: String,
    pub from_name: String,
    pub use_tls: bool,
}

/// Email service for sending verification codes
#[derive(Clone)]
pub struct EmailService {
    config: Option<EmailConfig>,
    codes: Arc<RwLock<HashMap<String, VerificationCode>>>,
    code_ttl_minutes: i64,
    max_attempts: u32,
    template_manager: Arc<EmailTemplateManager>,
}

impl std::fmt::Debug for EmailService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmailService")
            .field("configured", &self.config.is_some())
            .field("code_ttl_minutes", &self.code_ttl_minutes)
            .field("max_attempts", &self.max_attempts)
            .finish()
    }
}

impl EmailService {
    /// Create a new email service
    pub fn new(config: Option<EmailConfig>) -> Result<Self> {
        let template_manager = EmailTemplateManager::new()?;
        Ok(Self {
            config,
            codes: Arc::new(RwLock::new(HashMap::new())),
            code_ttl_minutes: 10, // 10 minutes default
            max_attempts: 3,
            template_manager: Arc::new(template_manager),
        })
    }

    /// Create with custom TTL
    pub fn with_ttl(config: Option<EmailConfig>, code_ttl_minutes: i64) -> Result<Self> {
        let template_manager = EmailTemplateManager::new()?;
        Ok(Self {
            config,
            codes: Arc::new(RwLock::new(HashMap::new())),
            code_ttl_minutes,
            max_attempts: 3,
            template_manager: Arc::new(template_manager),
        })
    }

    /// Generate a 6-digit verification code
    fn generate_code() -> String {
        let mut rng = rand::thread_rng();
        format!("{:06}", rng.gen_range(0..1_000_000))
    }

    /// Validate email format
    fn validate_email(email: &str) -> Result<()> {
        let email = email.trim();

        // Basic email validation
        if email.is_empty() || email.len() > 255 {
            return Err(Error::InvalidInput("Email length invalid".to_string()));
        }

        // Check for @ symbol
        if !email.contains('@') {
            return Err(Error::InvalidInput("Missing @ symbol".to_string()));
        }

        // Split and check parts
        let parts: Vec<&str> = email.split('@').collect();
        if parts.len() != 2 {
            return Err(Error::InvalidInput("Invalid format".to_string()));
        }

        let local = parts[0];
        let domain = parts[1];

        if local.is_empty() || domain.is_empty() {
            return Err(Error::InvalidInput("Empty local or domain".to_string()));
        }

        // Check domain has at least one dot
        if !domain.contains('.') {
            return Err(Error::InvalidInput("Invalid domain".to_string()));
        }

        Ok(())
    }

    /// Send verification code to email
    ///
    /// # Arguments
    /// * `email` - Email address to send code to
    ///
    /// # Returns
    /// The verification code (for testing purposes)
    pub async fn send_verification_code(&self, email: &str) -> Result<String> {
        // Validate email
        Self::validate_email(email)?;

        // Check if service is configured
        if self.config.is_none() {
            tracing::warn!("Email service not configured, returning code directly");
            let code = Self::generate_code();
            // Store code anyway
            let verification_code = VerificationCode {
                code: code.clone(),
                created_at: Utc::now(),
                attempts: 0,
            };
            self.codes.write().await.insert(email.to_string(), verification_code);
            return Ok(code);
        }

        // Generate code
        let code = Self::generate_code();

        // Store code
        let verification_code = VerificationCode {
            code: code.clone(),
            created_at: Utc::now(),
            attempts: 0,
        };
        self.codes.write().await.insert(email.to_string(), verification_code);

        // Send email
        if let Some(config) = &self.config {
            if let Err(e) = self.send_email(config, email, &code).await {
                tracing::error!("Failed to send email: {}", e);
                return Err(Error::Internal(format!("Failed to send email: {e}")));
            }
        }

        Ok(code)
    }

    /// Verify code for email
    pub async fn verify_code(&self, email: &str, code: &str) -> Result<()> {
        let mut codes = self.codes.write().await;

        let verification_code = codes
            .get_mut(email)
            .ok_or_else(|| Error::InvalidInput("No verification code found".to_string()))?;

        // Check if expired
        let expiration = verification_code.created_at + Duration::minutes(self.code_ttl_minutes);
        if Utc::now() > expiration {
            codes.remove(email);
            return Err(Error::InvalidInput("Verification code expired".to_string()));
        }

        // Check attempts
        verification_code.attempts += 1;
        if verification_code.attempts > self.max_attempts {
            codes.remove(email);
            return Err(Error::InvalidInput("Too many failed attempts".to_string()));
        }

        // Verify code
        if verification_code.code != code {
            return Err(Error::InvalidInput("Invalid verification code".to_string()));
        }

        // Remove code after successful verification
        codes.remove(email);

        Ok(())
    }

    /// Send email using SMTP (deprecated - use send_email_impl instead)
    #[allow(dead_code)]
    async fn send_email(&self, config: &EmailConfig, to: &str, code: &str) -> std::result::Result<(), EmailError> {
        let subject = "SyncTV - Verification Code";
        let body = format!(
            "SyncTV Email Verification\n\nYour verification code is: {}\n\nThis code will expire in {} minutes.\nIf you didn't request this code, please ignore this email.",
            code, self.code_ttl_minutes
        );

        self.send_email_impl(config, to, subject, &body).await
    }

    /// Send verification email
    ///
    /// Generates a token and sends verification email
    /// Returns the token (for testing)
    pub async fn send_verification_email(
        &self,
        email: &str,
        token_service: &EmailTokenService,
        user_id: &crate::models::UserId,
    ) -> Result<String> {
        // Validate email
        Self::validate_email(email)?;

        // Generate token
        let token = token_service
            .generate_token(user_id, EmailTokenType::EmailVerification)
            .await?;

        // Send email
        if let Some(config) = &self.config {
            if let Err(e) = self
                .send_verification_email_impl(config, email, &token)
                .await
            {
                tracing::error!("Failed to send verification email: {}", e);
                return Err(Error::Internal(format!("Failed to send email: {e}")));
            }
        } else {
            tracing::warn!("Email service not configured, returning token directly");
        }

        tracing::info!("Sent verification email to {}", email);
        Ok(token)
    }

    /// Send password reset email
    ///
    /// Generates a token and sends password reset email
    /// Returns the token (for testing)
    pub async fn send_password_reset_email(
        &self,
        email: &str,
        token_service: &EmailTokenService,
        user_id: &crate::models::UserId,
    ) -> Result<String> {
        // Validate email
        Self::validate_email(email)?;

        // Generate token
        let token = token_service
            .generate_token(user_id, EmailTokenType::PasswordReset)
            .await?;

        // Send email
        if let Some(config) = &self.config {
            if let Err(e) = self
                .send_password_reset_email_impl(config, email, &token)
                .await
            {
                tracing::error!("Failed to send password reset email: {}", e);
                return Err(Error::Internal(format!("Failed to send email: {e}")));
            }
        } else {
            tracing::warn!("Email service not configured, returning token directly");
        }

        tracing::info!("Sent password reset email to {}", email);
        Ok(token)
    }

    /// Send a test email to verify email configuration
    ///
    /// This is used by admins to test email settings
    pub async fn send_test_email(&self, to: &str) -> Result<()> {
        // Validate email
        Self::validate_email(to)?;

        // Check if service is configured
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::Internal("Email service not configured".to_string()))?;

        // Render template
        let sent_at = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();
        let (html_body, plain_text_body) = self.template_manager
            .render_test_email(&config.smtp_host, config.smtp_port, &sent_at)
            .map_err(|e| Error::Internal(format!("Failed to render template: {e}")))?;

        // Send test email
        let subject = "SyncTV Email Test";
        self.send_html_email(config, to, subject, &html_body, &plain_text_body)
            .await
            .map_err(|e| Error::Internal(format!("Failed to send test email: {e}")))?;

        tracing::info!("Sent test email to {}", to);
        Ok(())
    }

    /// Send verification email implementation
    async fn send_verification_email_impl(
        &self,
        config: &EmailConfig,
        to: &str,
        token: &str,
    ) -> std::result::Result<(), EmailError> {
        let subject = "Verify your SyncTV email";
        let (html_body, plain_text_body) = self.template_manager
            .render_verification_email(token, "24 hours")
            .map_err(|e| EmailError::SendError(format!("Failed to render template: {e}")))?;

        self.send_html_email(config, to, subject, &html_body, &plain_text_body).await
    }

    /// Send password reset email implementation
    async fn send_password_reset_email_impl(
        &self,
        config: &EmailConfig,
        to: &str,
        token: &str,
    ) -> std::result::Result<(), EmailError> {
        let subject = "Reset your SyncTV password";
        let (html_body, plain_text_body) = self.template_manager
            .render_password_reset_email(token, "1 hour")
            .map_err(|e| EmailError::SendError(format!("Failed to render template: {e}")))?;

        self.send_html_email(config, to, subject, &html_body, &plain_text_body).await
    }

    /// Send plain text email using SMTP
    async fn send_email_impl(
        &self,
        config: &EmailConfig,
        to: &str,
        subject: &str,
        body: &str,
    ) -> std::result::Result<(), EmailError> {
        // Parse email addresses
        let from_mailbox: Mailbox = format!("{} <{}>", config.from_name, config.from_email)
            .parse()
            .map_err(|e| EmailError::SendError(format!("Invalid from address: {e}")))?;

        let to_mailbox: Mailbox = to
            .parse()
            .map_err(|e| EmailError::SendError(format!("Invalid to address: {e}")))?;

        // Build message
        let email = Message::builder()
            .from(from_mailbox)
            .to(to_mailbox)
            .subject(subject)
            .body(body.to_string())
            .map_err(|e| EmailError::SendError(format!("Failed to build email: {e}")))?;

        // Send email
        self.send_message(config, email).await
    }

    /// Send HTML email with plain text fallback
    async fn send_html_email(
        &self,
        config: &EmailConfig,
        to: &str,
        subject: &str,
        html_body: &str,
        plain_text_body: &str,
    ) -> std::result::Result<(), EmailError> {
        // Parse email addresses
        let from_mailbox: Mailbox = format!("{} <{}>", config.from_name, config.from_email)
            .parse()
            .map_err(|e| EmailError::SendError(format!("Invalid from address: {e}")))?;

        let to_mailbox: Mailbox = to
            .parse()
            .map_err(|e| EmailError::SendError(format!("Invalid to address: {e}")))?;

        // Build multipart message (HTML + plain text fallback)
        let email = Message::builder()
            .from(from_mailbox)
            .to(to_mailbox)
            .subject(subject)
            .multipart(
                MultiPart::alternative()
                    .singlepart(
                        lettre::message::SinglePart::builder()
                            .header(ContentType::TEXT_PLAIN)
                            .body(plain_text_body.to_string())
                    )
                    .singlepart(
                        lettre::message::SinglePart::builder()
                            .header(ContentType::TEXT_HTML)
                            .body(html_body.to_string())
                    )
            )
            .map_err(|e| EmailError::SendError(format!("Failed to build email: {e}")))?;

        // Send email
        self.send_message(config, email).await
    }

    /// Send email message via SMTP
    async fn send_message(
        &self,
        config: &EmailConfig,
        email: Message,
    ) -> std::result::Result<(), EmailError> {
        // Get recipient before consuming email
        let recipient = email.envelope().to()[0].clone();

        // Create SMTP credentials
        let creds = Credentials::new(
            config.smtp_username.clone(),
            config.smtp_password.clone(),
        );

        // Create SMTP transport
        let transport = if config.use_tls {
            AsyncSmtpTransport::<Tokio1Executor>::relay(&config.smtp_host)
                .map_err(|e| EmailError::SendError(format!("Failed to create SMTP transport: {e}")))?
                .credentials(creds)
                .port(config.smtp_port)
                .build()
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.smtp_host)
                .credentials(creds)
                .port(config.smtp_port)
                .build()
        };

        // Send email
        transport
            .send(email)
            .await
            .map_err(|e| EmailError::SendError(format!("Failed to send email: {e}")))?;

        tracing::info!(
            "Email sent successfully to {} via SMTP {}:{}",
            recipient,
            config.smtp_host,
            config.smtp_port
        );

        Ok(())
    }

    /// Clean up expired codes
    pub async fn cleanup_expired_codes(&self) {
        let mut codes = self.codes.write().await;
        let now = Utc::now();
        let expiration_threshold = now - Duration::minutes(self.code_ttl_minutes);

        codes.retain(|_, code| code.created_at > expiration_threshold);
    }

    /// Check if email service is configured
    #[must_use] 
    pub const fn is_configured(&self) -> bool {
        self.config.is_some()
    }
}

impl Default for EmailService {
    fn default() -> Self {
        Self::new(None).expect("Failed to create default EmailService")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_code() {
        let code = EmailService::generate_code();
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn test_validate_email_valid() {
        assert!(EmailService::validate_email("test@example.com").is_ok());
        assert!(EmailService::validate_email("user.name+tag@domain.co.uk").is_ok());
    }

    #[test]
    fn test_validate_email_invalid() {
        assert!(EmailService::validate_email("").is_err());
        assert!(EmailService::validate_email("invalid").is_err());
        assert!(EmailService::validate_email("@example.com").is_err());
        assert!(EmailService::validate_email("test@").is_err());
        assert!(EmailService::validate_email("test@.com").is_err());
    }

    #[tokio::test]
    async fn test_send_and_verify_code() {
        let service = EmailService::new(None).unwrap();

        let email = "test@example.com";
        let code = service.send_verification_code(email).await.unwrap();

        // Verify correct code
        assert!(service.verify_code(email, &code).await.is_ok());

        // Verify wrong code
        assert!(service.verify_code(email, "000000").await.is_err());

        // Verify again after successful verification
        assert!(service.verify_code(email, &code).await.is_err());
    }

    #[tokio::test]
    async fn test_verify_expired_code() {
        let service = EmailService::with_ttl(None, -1).unwrap(); // Expired immediately

        let email = "test@example.com";
        let code = service.send_verification_code(email).await.unwrap();

        // Wait a bit to ensure expiration
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        assert!(service.verify_code(email, &code).await.is_err());
    }

    #[tokio::test]
    async fn test_max_attempts() {
        let service = EmailService::with_ttl(None, 60).unwrap();

        let email = "test@example.com";
        let code = service.send_verification_code(email).await.unwrap();

        // Try wrong codes up to max attempts
        for _ in 0..3 {
            assert!(service.verify_code(email, "000000").await.is_err());
        }

        // After max attempts, even correct code should fail
        assert!(service.verify_code(email, &code).await.is_err());
    }
}
