use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use super::id::UserId;

/// Global user role (design document 06/07: role and status separation)
///
/// This represents the user's permission level at the GLOBAL level,
/// independent of their account status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    /// Root user (super administrator)
    /// - Can manage all admins
    /// - Can access all rooms
    /// - Can modify global settings
    Root,

    /// Platform administrator
    /// - Can manage regular users (approve, ban, delete)
    /// - Can manage rooms (approve, ban, delete)
    /// - Cannot manage Root users
    Admin,

    /// Regular user
    /// - Can create rooms (subject to global config)
    /// - Can join rooms
    User,
}

impl UserRole {
    /// Check if this role can manage another role
    #[must_use] 
    pub const fn can_manage(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Root, _) => true,
            (Self::Admin, Self::User) => true,
            _ => false,
        }
    }

    /// Check if this role is admin or above
    #[must_use] 
    pub const fn is_admin_or_above(&self) -> bool {
        matches!(self, Self::Root | Self::Admin)
    }

    #[must_use] 
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Admin => "admin",
            Self::User => "user",
        }
    }
}

impl FromStr for UserRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "root" => Ok(Self::Root),
            "admin" => Ok(Self::Admin),
            "user" => Ok(Self::User),
            _ => Err(format!("Unknown user role: {s}")),
        }
    }
}

impl std::fmt::Display for UserRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// User account status (design document 06: role and status separation)
///
/// This represents the user's ACCOUNT state, independent of their role.
/// A user can be Active/Pending/Banned regardless of their Role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserStatus {
    /// Normal active state
    /// - Can login and use all features
    Active,

    /// Pending approval
    /// - Can login but cannot create or join rooms
    Pending,

    /// Banned state
    /// - Cannot login
    /// - All operations forbidden
    Banned,
}

impl UserStatus {
    #[must_use] 
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Pending => "pending",
            Self::Banned => "banned",
        }
    }

    /// Check if user can login with this status
    #[must_use] 
    pub const fn can_login(&self) -> bool {
        matches!(self, Self::Active | Self::Pending)
    }

    /// Check if user can create rooms with this status
    #[must_use] 
    pub const fn can_create_room(&self) -> bool {
        matches!(self, Self::Active)
    }

    /// Check if user can join rooms with this status
    #[must_use] 
    pub const fn can_join_room(&self) -> bool {
        matches!(self, Self::Active)
    }

    #[must_use] 
    pub const fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }

    #[must_use] 
    pub const fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }

    #[must_use] 
    pub const fn is_banned(&self) -> bool {
        matches!(self, Self::Banned)
    }
}

impl FromStr for UserStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "active" => Ok(Self::Active),
            "pending" => Ok(Self::Pending),
            "banned" => Ok(Self::Banned),
            _ => Err(format!("Unknown user status: {s}")),
        }
    }
}

impl std::fmt::Display for UserStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// User signup method
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignupMethod {
    Email,
    OAuth2,
}

impl SignupMethod {
    #[must_use] 
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::OAuth2 => "oauth2",
        }
    }

    /// Parse signup method from string name (defaults to email for unknown values)
    #[must_use] 
    pub fn from_str_name(s: &str) -> Self {
        match s {
            "email" => Self::Email,
            "oauth2" => Self::OAuth2,
            _ => Self::Email, // Default to email
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

    /// User RBAC role (global access level) - SEPARATE from status
    pub role: UserRole,

    /// User status (account state) - SEPARATE from role
    pub status: UserStatus,

    pub signup_method: Option<SignupMethod>,  // NULL for legacy users
    pub email_verified: bool,  // Whether email has been verified
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl User {
    #[must_use] 
    pub fn new(username: String, email: Option<String>, password_hash: String, signup_method: Option<SignupMethod>) -> Self {
        let now = Utc::now();
        Self {
            id: UserId::new(),
            username,
            email,
            password_hash,
            role: UserRole::User,  // Default role
            status: UserStatus::Pending,  // Default status (requires email verification)
            signup_method,
            email_verified: false,  // Default to not verified
            created_at: now,
            updated_at: now,
            deleted_at: None,
        }
    }

    #[must_use] 
    pub const fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }

    /// Check if user has specific role level (RBAC)
    #[must_use] 
    pub const fn is_root(&self) -> bool {
        matches!(self.role, UserRole::Root)
    }

    #[must_use] 
    pub const fn is_admin(&self) -> bool {
        matches!(self.role, UserRole::Admin)
    }

    #[must_use] 
    pub const fn is_admin_or_above(&self) -> bool {
        self.role.is_admin_or_above()
    }

    /// Check if user can login (checks status, not role)
    #[must_use] 
    pub const fn can_login(&self) -> bool {
        self.status.can_login()
    }

    /// Check if user can create rooms (checks both role and status)
    #[must_use] 
    pub const fn can_create_room(&self, allow_user: bool) -> bool {
        if !self.status.can_create_room() {
            return false;
        }

        match self.role {
            UserRole::Root | UserRole::Admin => true,
            UserRole::User => allow_user,
        }
    }

    /// Check if user can join rooms (checks status)
    #[must_use] 
    pub const fn can_join_room(&self) -> bool {
        self.status.can_join_room()
    }

    /// Check if user can unbind a provider
    /// `OAuth2` users cannot remove all `OAuth2` providers unless they have email
    /// Email users cannot remove their email
    #[must_use] 
    pub const fn can_unbind_provider(&self, has_oauth2_count: usize, has_email: bool) -> bool {
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
