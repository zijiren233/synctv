//! Input validation using mature crates
//!
//! This module provides production-grade input validation using the `validator` crate.

use std::sync::LazyLock;

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
}

impl Default for UsernameValidator {
    fn default() -> Self {
        Self {
            min_length: 3,
            max_length: 50,
        }
    }
}

impl UsernameValidator {
    #[must_use] 
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use] 
    pub const fn with_length(mut self, min: usize, max: usize) -> Self {
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
        if let Some(first_char) = username.chars().next() {
            if first_char == '_' || first_char == '-' {
                return Err(ValidationError::Field {
                    field: "username".to_string(),
                    message: "cannot start with underscore or hyphen".to_string(),
                });
            }
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
    #[must_use] 
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use] 
    pub const fn with_min_length(mut self, length: usize) -> Self {
        self.min_length = length;
        self
    }

    #[must_use] 
    pub const fn require_special_char(mut self, required: bool) -> Self {
        self.require_special_char = required;
        self
    }

    /// Maximum password length to prevent bcrypt `DoS` (bcrypt input limit is 72 bytes)
    const MAX_LENGTH: usize = 128;

    pub fn validate(&self, password: &str) -> ValidationResult<()> {
        // Check length
        if password.len() < self.min_length {
            return Err(ValidationError::Field {
                field: "password".to_string(),
                message: format!("must be at least {} characters", self.min_length),
            });
        }

        if password.len() > Self::MAX_LENGTH {
            return Err(ValidationError::Field {
                field: "password".to_string(),
                message: format!("must not exceed {} characters", Self::MAX_LENGTH),
            });
        }

        // Check for uppercase
        if self.require_uppercase && !password.chars().any(char::is_uppercase) {
            return Err(ValidationError::Field {
                field: "password".to_string(),
                message: "must contain at least one uppercase letter".to_string(),
            });
        }

        // Check for lowercase
        if self.require_lowercase && !password.chars().any(char::is_lowercase) {
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

/// Pre-compiled email validation regex
static EMAIL_REGEX: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$")
        .expect("email validation regex is invalid")
});

/// Email validator
#[derive(Default)]
pub struct EmailValidator {}


impl EmailValidator {
    #[must_use] 
    pub fn new() -> Self {
        Self::default()
    }

    pub fn validate(&self, email: &str) -> ValidationResult<()> {
        if !EMAIL_REGEX.is_match(email) {
            return Err(ValidationError::Field {
                field: "email".to_string(),
                message: "must be a valid email address".to_string(),
            });
        }

        Ok(())
    }
}

/// URL validator
#[derive(Default)]
pub struct UrlValidator {
    allow_https_only: bool,
    allowed_domains: Option<Vec<String>>,
}


impl UrlValidator {
    #[must_use] 
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use] 
    pub const fn https_only(mut self) -> Self {
        self.allow_https_only = true;
        self
    }

    #[must_use] 
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
                        if !domains.iter().any(|d| host == d.as_str() || host.ends_with(&format!(".{d}"))) {
                            return Err(ValidationError::Field {
                                field: "url".to_string(),
                                message: format!("domain not in allowed list: {domains:?}"),
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
    #[must_use] 
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use] 
    pub const fn with_length(mut self, min: usize, max: usize) -> Self {
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
        if name.chars().any(char::is_control) {
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
    #[must_use] 
    pub const fn new() -> Self {
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

    #[must_use] 
    pub const fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn into_result(self) -> ValidationResult<()> {
        let mut errors = self.errors;
        match errors.len() {
            0 => Ok(()),
            1 => Err(errors.pop().expect("length checked")),
            _ => {
                let messages: Vec<String> = errors.iter()
                    .map(std::string::ToString::to_string)
                    .collect();
                Err(ValidationError::Multiple(messages.join("; ")))
            }
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

    #[test]
    fn test_username_edge_cases() {
        let validator = UsernameValidator::new();

        // Empty username
        assert!(validator.validate("").is_err());

        // Exactly minimum length
        assert!(validator.validate("abc").is_ok());

        // Exactly maximum length (50 chars)
        let max_length = "a".repeat(50);
        assert!(validator.validate(&max_length).is_ok());

        // Over maximum length
        let too_long = "a".repeat(51);
        assert!(validator.validate(&too_long).is_err());

        // Starts with hyphen
        assert!(validator.validate("-invalid").is_err());

        // Unicode characters (should be valid as they are alphanumeric)
        assert!(validator.validate("用户名").is_ok());

        // Mixed valid characters
        assert!(validator.validate("User-Name_123").is_ok());
    }

    #[test]
    fn test_password_edge_cases() {
        let validator = PasswordValidator::new();

        // Empty password
        assert!(validator.validate("").is_err());

        // Exactly minimum length
        assert!(validator.validate("Abcd1234").is_ok());

        // Maximum length (128 chars)
        let max_password = "A".repeat(64) + "a" + &"1".repeat(63);
        assert!(validator.validate(&max_password).is_ok());

        // Over maximum length
        let too_long = "A".repeat(64) + "a" + &"1".repeat(64);
        assert!(validator.validate(&too_long).is_err());

        // With special characters
        let validator_with_special = PasswordValidator::new().require_special_char(true);
        assert!(validator_with_special.validate("Password123!").is_ok());
        assert!(validator_with_special.validate("Password123").is_err());

        // Relaxed requirements
        let relaxed_validator = PasswordValidator::new()
            .with_min_length(4)
            .require_special_char(false);
        assert!(relaxed_validator.validate("Abc1").is_ok());
    }

    #[test]
    fn test_room_name_validation() {
        let validator = RoomNameValidator::new();

        // Valid room names
        assert!(validator.validate("My Room").is_ok());
        assert!(validator.validate("a").is_ok());
        assert!(validator.validate("Room-123_Test").is_ok());

        // Invalid room names
        assert!(validator.validate("").is_err()); // Empty

        // Control characters
        assert!(validator.validate("Room\x00Name").is_err());
        assert!(validator.validate("Room\nName").is_err());

        // Too long
        let too_long = "a".repeat(101);
        assert!(validator.validate(&too_long).is_err());
    }

    #[test]
    fn test_email_edge_cases() {
        let validator = EmailValidator::new();

        // Valid edge cases
        assert!(validator.validate("a@b.co").is_ok());
        assert!(validator.validate("user+tag@example.com").is_ok());
        assert!(validator.validate("user@sub.domain.example.com").is_ok());

        // Invalid edge cases
        assert!(validator.validate("").is_err());
        assert!(validator.validate("user@.com").is_err());
        assert!(validator.validate("user@example").is_err()); // No TLD
        assert!(validator.validate("user@example.c").is_err()); // TLD too short
    }

    #[test]
    fn test_url_edge_cases() {
        let validator = UrlValidator::new();

        // Both HTTP and HTTPS allowed
        assert!(validator.validate("http://example.com").is_ok());
        assert!(validator.validate("https://example.com").is_ok());

        // HTTPS only
        let https_only = UrlValidator::new().https_only();
        assert!(https_only.validate("https://example.com").is_ok());
        assert!(https_only.validate("http://example.com").is_err());

        // Domain whitelist
        let domain_validator = UrlValidator::new()
            .with_allowed_domains(vec!["example.com".to_string(), "trusted.org".to_string()]);
        assert!(domain_validator.validate("https://example.com/path").is_ok());
        assert!(domain_validator.validate("https://sub.example.com/path").is_ok());
        assert!(domain_validator.validate("https://other.com").is_err());

        // Invalid URLs
        assert!(validator.validate("").is_err());
        assert!(validator.validate("not-a-url").is_err());
        assert!(validator.validate("ftp://example.com").is_ok()); // ftp is a valid scheme
    }

    #[test]
    fn test_validation_error_messages() {
        let validator = UsernameValidator::new();
        let err = validator.validate("ab").unwrap_err();
        assert!(err.to_string().contains("username"));
        assert!(err.to_string().contains("3"));

        let validator = PasswordValidator::new();
        let err = validator.validate("weak").unwrap_err();
        assert!(err.to_string().contains("password"));
        assert!(err.to_string().contains("8"));
    }

    #[test]
    fn test_batch_validation_multiple_errors() {
        let mut validator = Validator::new();

        validator
            .validate_field("username", UsernameValidator::new().validate("ab")) // Too short
            .validate_field("email", EmailValidator::new().validate("invalid")) // Invalid email
            .validate_field("password", PasswordValidator::new().validate("weak")); // Too short

        let result = validator.into_result();
        assert!(result.is_err());

        match result {
            Err(ValidationError::Multiple(msgs)) => {
                assert!(msgs.contains("username"));
                assert!(msgs.contains("email"));
                assert!(msgs.contains("password"));
            }
            _ => panic!("Expected Multiple errors"),
        }
    }
}
