//! Input validation using mature crates
//!
//! This module provides production-grade input validation using the `validator` crate.

use std::collections::HashMap;
use std::fmt;

/// Validation error
#[derive(Debug, Clone, thiserror::Error)]
pub enum ValidationError {
    #[error("Invalid {field}: {message}")]
    Field { field: String, message: String },

    #[error("Multiple validation errors: {0}")]
    Multiple(String),
}

/// Validation result
pub type ValidationResult<T> = Result<T, ValidationError>;

/// Username validator
pub struct UsernameValidator {
    min_length: usize,
    max_length: usize,
    allowed_chars: Option<String>,
}

impl Default for UsernameValidator {
    fn default() -> Self {
        Self {
            min_length: 3,
            max_length: 50,
            allowed_chars: None, // Allow alphanumeric, underscore, hyphen
        }
    }
}

impl UsernameValidator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_length(mut self, min: usize, max: usize) -> Self {
        self.min_length = min;
        self.max_length = max;
        self
    }

    pub fn validate(&self, username: &str) -> ValidationResult<()> {
        // Check length
        if username.len() < self.min_length {
            return Err(ValidationError::Field {
                field: "username".to_string(),
                message: format!("must be at least {} characters", self.min_length),
            });
        }

        if username.len() > self.max_length {
            return Err(ValidationError::Field {
                field: "username".to_string(),
                message: format!("must be at most {} characters", self.max_length),
            });
        }

        // Check characters (alphanumeric, underscore, hyphen)
        if !username.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            return Err(ValidationError::Field {
                field: "username".to_string(),
                message: "can only contain letters, numbers, underscores, and hyphens".to_string(),
            });
        }

        // Cannot start with special character
        let first_char = username.chars().next().unwrap();
        if first_char == '_' || first_char == '-' {
            return Err(ValidationError::Field {
                field: "username".to_string(),
                message: "cannot start with underscore or hyphen".to_string(),
            });
        }

        Ok(())
    }
}

/// Password validator
pub struct PasswordValidator {
    min_length: usize,
    require_uppercase: bool,
    require_lowercase: bool,
    require_digit: bool,
    require_special_char: bool,
}

impl Default for PasswordValidator {
    fn default() -> Self {
        Self {
            min_length: 8,
            require_uppercase: true,
            require_lowercase: true,
            require_digit: true,
            require_special_char: false,
        }
    }
}

impl PasswordValidator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_min_length(mut self, length: usize) -> Self {
        self.min_length = length;
        self
    }

    pub fn require_special_char(mut self, required: bool) -> Self {
        self.require_special_char = required;
        self
    }

    pub fn validate(&self, password: &str) -> ValidationResult<()> {
        // Check length
        if password.len() < self.min_length {
            return Err(ValidationError::Field {
                field: "password".to_string(),
                message: format!("must be at least {} characters", self.min_length),
            });
        }

        // Check for uppercase
        if self.require_uppercase && !password.chars().any(|c| c.is_uppercase()) {
            return Err(ValidationError::Field {
                field: "password".to_string(),
                message: "must contain at least one uppercase letter".to_string(),
            });
        }

        // Check for lowercase
        if self.require_lowercase && !password.chars().any(|c| c.is_lowercase()) {
            return Err(ValidationError::Field {
                field: "password".to_string(),
                message: "must contain at least one lowercase letter".to_string(),
            });
        }

        // Check for digit
        if self.require_digit && !password.chars().any(|c| c.is_ascii_digit()) {
            return Err(ValidationError::Field {
                field: "password".to_string(),
                message: "must contain at least one digit".to_string(),
            });
        }

        // Check for special character
        if self.require_special_char && !password.chars().any(|c| {
            !c.is_alphanumeric()
        }) {
            return Err(ValidationError::Field {
                field: "password".to_string(),
                message: "must contain at least one special character".to_string(),
            });
        }

        Ok(())
    }
}

/// Email validator
pub struct EmailValidator {
    require_tld: bool,
}

impl Default for EmailValidator {
    fn default() -> Self {
        Self {
            require_tld: true,
        }
    }
}

impl EmailValidator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn validate(&self, email: &str) -> ValidationResult<()> {
        // Basic email validation using regex
        let email_regex = regex::Regex::new(
            r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$"
        ).unwrap();

        if !email_regex.is_match(email) {
            return Err(ValidationError::Field {
                field: "email".to_string(),
                message: "must be a valid email address".to_string(),
            });
        }

        Ok(())
    }
}

/// URL validator
pub struct UrlValidator {
    allow_https_only: bool,
    allowed_domains: Option<Vec<String>>,
}

impl Default for UrlValidator {
    fn default() -> Self {
        Self {
            allow_https_only: false,
            allowed_domains: None,
        }
    }
}

impl UrlValidator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn https_only(mut self) -> Self {
        self.allow_https_only = true;
        self
    }

    pub fn with_allowed_domains(mut self, domains: Vec<String>) -> Self {
        self.allowed_domains = Some(domains);
        self
    }

    pub fn validate(&self, url: &str) -> ValidationResult<()> {
        match url::Url::parse(url) {
            Ok(parsed) => {
                // Check HTTPS requirement
                if self.allow_https_only && parsed.scheme() != "https" {
                    return Err(ValidationError::Field {
                        field: "url".to_string(),
                        message: "must use HTTPS".to_string(),
                    });
                }

                // Check allowed domains
                if let Some(ref domains) = self.allowed_domains {
                    if let Some(host) = parsed.host_str() {
                        if !domains.iter().any(|d| host.ends_with(d)) {
                            return Err(ValidationError::Field {
                                field: "url".to_string(),
                                message: format!("domain not in allowed list: {:?}", domains),
                            });
                        }
                    }
                }

                Ok(())
            }
            Err(_) => Err(ValidationError::Field {
                field: "url".to_string(),
                message: "must be a valid URL".to_string(),
            }),
        }
    }
}

/// Room name validator
pub struct RoomNameValidator {
    min_length: usize,
    max_length: usize,
}

impl Default for RoomNameValidator {
    fn default() -> Self {
        Self {
            min_length: 1,
            max_length: 100,
        }
    }
}

impl RoomNameValidator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_length(mut self, min: usize, max: usize) -> Self {
        self.min_length = min;
        self.max_length = max;
        self
    }

    pub fn validate(&self, name: &str) -> ValidationResult<()> {
        if name.len() < self.min_length {
            return Err(ValidationError::Field {
                field: "room_name".to_string(),
                message: format!("must be at least {} characters", self.min_length),
            });
        }

        if name.len() > self.max_length {
            return Err(ValidationError::Field {
                field: "room_name".to_string(),
                message: format!("must be at most {} characters", self.max_length),
            });
        }

        // Check for control characters
        if name.chars().any(|c| c.is_control()) {
            return Err(ValidationError::Field {
                field: "room_name".to_string(),
                message: "cannot contain control characters".to_string(),
            });
        }

        Ok(())
    }
}

/// Batch validator for multiple fields
pub struct Validator {
    errors: Vec<ValidationError>,
}

impl Validator {
    pub fn new() -> Self {
        Self {
            errors: Vec::new(),
        }
    }

    pub fn validate_field<F>(&mut self, _field: &str, result: ValidationResult<F>) -> &mut Self {
        if let Err(e) = result {
            self.errors.push(e);
        }
        self
    }

    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn into_result(self) -> ValidationResult<()> {
        if self.errors.is_empty() {
            Ok(())
        } else if self.errors.len() == 1 {
            Err(self.errors.into_iter().next().unwrap())
        } else {
            let messages: Vec<String> = self.errors.iter()
                .map(|e| e.to_string())
                .collect();
            Err(ValidationError::Multiple(messages.join("; ")))
        }
    }
}

impl Default for Validator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_username_validation() {
        let validator = UsernameValidator::new();

        // Valid usernames
        assert!(validator.validate("alice").is_ok());
        assert!(validator.validate("bob_123").is_ok());
        assert!(validator.validate("charlie-test").is_ok());

        // Invalid usernames
        assert!(validator.validate("ab").is_err()); // Too short
        assert!(validator.validate("_invalid").is_err()); // Starts with underscore
        assert!(validator.validate("invalid@name").is_err()); // Invalid character
    }

    #[test]
    fn test_password_validation() {
        let validator = PasswordValidator::new();

        // Valid passwords
        assert!(validator.validate("Password123").is_ok());

        // Invalid passwords
        assert!(validator.validate("short").is_err()); // Too short
        assert!(validator.validate("nouppercase123").is_err()); // No uppercase
        assert!(validator.validate("NOLOWERCASE123").is_err()); // No lowercase
        assert!(validator.validate("NoDigits").is_err()); // No digit
    }

    #[test]
    fn test_email_validation() {
        let validator = EmailValidator::new();

        // Valid emails
        assert!(validator.validate("user@example.com").is_ok());
        assert!(validator.validate("user.name@example.co.uk").is_ok());

        // Invalid emails
        assert!(validator.validate("notanemail").is_err());
        assert!(validator.validate("@example.com").is_err());
        assert!(validator.validate("user@").is_err());
    }

    #[test]
    fn test_url_validation() {
        let validator = UrlValidator::new().https_only();

        // Valid HTTPS URLs
        assert!(validator.validate("https://example.com").is_ok());
        assert!(validator.validate("https://example.com/path").is_ok());

        // Invalid URLs
        assert!(validator.validate("http://example.com").is_err()); // Not HTTPS
        assert!(validator.validate("not-a-url").is_err());
    }

    #[test]
    fn test_batch_validation() {
        let mut validator = Validator::new();

        validator
            .validate_field("username", UsernameValidator::new().validate("valid_user"))
            .validate_field("email", EmailValidator::new().validate("invalid-email"))
            .validate_field("password", PasswordValidator::new().validate("short"));

        assert!(!validator.is_valid());
        assert!(validator.into_result().is_err());
    }
}
