use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::id::UserId;
use super::permission::PermissionBits;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub username: String,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub permissions: PermissionBits,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl User {
    pub fn new(username: String, email: String, password_hash: String) -> Self {
        let now = Utc::now();
        Self {
            id: UserId::new(),
            username,
            email,
            password_hash,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateUserRequest {
    pub username: Option<String>,
    pub email: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserListQuery {
    pub page: i32,
    pub page_size: i32,
    pub search: Option<String>,
}

impl Default for UserListQuery {
    fn default() -> Self {
        Self {
            page: 1,
            page_size: 20,
            search: None,
        }
    }
}
