use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::id::UserId;
use super::permission::PermissionBits;

/// User signup method
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignupMethod {
    Email,
    OAuth2,
}

impl SignupMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            SignupMethod::Email => "email",
            SignupMethod::OAuth2 => "oauth2",
        }
    }

    /// Parse signup method from string name (defaults to email for unknown values)
    pub fn from_str_name(s: &str) -> Self {
        match s {
            "email" => SignupMethod::Email,
            "oauth2" => SignupMethod::OAuth2,
            _ => SignupMethod::Email, // Default to email
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub username: String,
    pub email: Option<String>,  // NULL allowed for OAuth2 users
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub signup_method: Option<SignupMethod>,  // NULL for legacy users
    pub email_verified: bool,  // Whether email has been verified
    pub permissions: PermissionBits,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl User {
    pub fn new(username: String, email: Option<String>, password_hash: String, signup_method: Option<SignupMethod>) -> Self {
        let now = Utc::now();
        Self {
            id: UserId::new(),
            username,
            email,
            password_hash,
            signup_method,
            email_verified: false,  // Default to not verified
            permissions: PermissionBits::empty(),
            created_at: now,
            updated_at: now,
            deleted_at: None,
        }
    }

    pub fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }

    pub fn has_permission(&self, permission: i64) -> bool {
        self.permissions.has(permission)
    }

    /// Check if user can unbind a provider
    /// OAuth2 users cannot remove all OAuth2 providers unless they have email
    /// Email users cannot remove their email
    pub fn can_unbind_provider(&self, has_oauth2_count: usize, has_email: bool) -> bool {
        match self.signup_method {
            None => {
                // Legacy users - allow if they have email or multiple OAuth2
                has_email || has_oauth2_count > 1
            }
            Some(SignupMethod::Email) => {
                // Email users can unbind OAuth2, but need to keep email
                true
            }
            Some(SignupMethod::OAuth2) => {
                // OAuth2 users must keep at least one OAuth2 or add email
                has_oauth2_count > 1 || has_email
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub email: Option<String>,  // Optional email
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateUserRequest {
    pub username: Option<String>,
    pub email: Option<Option<String>>,  // Option<Option<String>>: Some(None) means set to NULL, None means don't update
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserListQuery {
    pub page: i32,
    pub page_size: i32,
    pub search: Option<String>,
    pub status: Option<String>, // "active", "banned", etc.
    pub role: Option<String>,   // "user", "admin", "root"
}

impl Default for UserListQuery {
    fn default() -> Self {
        Self {
            page: 1,
            page_size: 20,
            search: None,
            status: None,
            role: None,
        }
    }
}
