use chrono::Utc;
use sqlx::{postgres::PgRow, PgPool, Row};
use std::str::FromStr;

use crate::{
    models::{SignupMethod, User, UserId, UserListQuery, UserRole, UserStatus},
    Error, Result,
};

/// User repository for database operations
#[derive(Clone)]
pub struct UserRepository {
    pool: PgPool,
}

impl UserRepository {
    #[must_use] 
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get the database pool
    #[must_use] 
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Create a new user
    pub async fn create(&self, user: &User) -> Result<User> {
        let row = sqlx::query(
            r"
            INSERT INTO users (id, username, email, password_hash, signup_method, role, status, email_verified, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING id, username, email, password_hash, signup_method, role, status, created_at, updated_at, deleted_at, email_verified
            ",
        )
        .bind(user.id.as_str())
        .bind(&user.username)
        .bind(user.email.as_ref())
        .bind(&user.password_hash)
        .bind(user.signup_method.map(|m| m.as_str()))
        .bind(user.role.as_str())
        .bind(user.status.as_str())
        .bind(user.email_verified)
        .bind(user.created_at)
        .bind(user.updated_at)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db_err) if db_err.constraint().is_some() => {
                Error::InvalidInput("User with this username or email already exists".to_string())
            }
            _ => Error::Database(e),
        })?;

        self.row_to_user(row)
    }

    /// Get user by ID
    pub async fn get_by_id(&self, user_id: &UserId) -> Result<Option<User>> {
        let row = sqlx::query(
            r"
            SELECT id, username, email, password_hash, signup_method, role, status, created_at, updated_at, deleted_at, email_verified
            FROM users
            WHERE id = $1 AND deleted_at IS NULL
            ",
        )
        .bind(user_id.as_str())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_user(row)?)),
            None => Ok(None),
        }
    }

    /// Get user by username
    pub async fn get_by_username(&self, username: &str) -> Result<Option<User>> {
        let row = sqlx::query(
            r"
            SELECT id, username, email, password_hash, signup_method, role, status, created_at, updated_at, deleted_at, email_verified
            FROM users
            WHERE username = $1 AND deleted_at IS NULL
            ",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_user(row)?)),
            None => Ok(None),
        }
    }

    /// Get user by email
    pub async fn get_by_email(&self, email: &str) -> Result<Option<User>> {
        let row = sqlx::query(
            r"
            SELECT id, username, email, password_hash, signup_method, role, status, created_at, updated_at, deleted_at, email_verified
            FROM users
            WHERE email = $1 AND deleted_at IS NULL
            ",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_user(row)?)),
            None => Ok(None),
        }
    }

    /// Update user
    pub async fn update(&self, user: &User) -> Result<User> {
        let row = sqlx::query(
            r"
            UPDATE users
            SET username = $2, email = $3, password_hash = $4, role = $5, status = $6, updated_at = $7
            WHERE id = $1 AND deleted_at IS NULL
            RETURNING id, username, email, password_hash, signup_method, role, status, created_at, updated_at, deleted_at, email_verified
            ",
        )
        .bind(user.id.as_str())
        .bind(&user.username)
        .bind(user.email.as_ref())
        .bind(&user.password_hash)
        .bind(user.role.as_str())
        .bind(user.status.as_str())
        .bind(Utc::now())
        .fetch_one(&self.pool)
        .await?;

        self.row_to_user(row)
    }

    /// Soft delete user
    pub async fn delete(&self, user_id: &UserId) -> Result<bool> {
        let result = sqlx::query(
            r"
            UPDATE users
            SET deleted_at = $2
            WHERE id = $1 AND deleted_at IS NULL
            ",
        )
        .bind(user_id.as_str())
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Update user password
    pub async fn update_password(&self, user_id: &UserId, password_hash: &str) -> Result<User> {
        let row = sqlx::query(
            r"
            UPDATE users
            SET password_hash = $2, updated_at = $3
            WHERE id = $1 AND deleted_at IS NULL
            RETURNING id, username, email, password_hash, signup_method, permissions, created_at, updated_at, deleted_at
            ",
        )
        .bind(user_id.as_str())
        .bind(password_hash)
        .bind(Utc::now())
        .fetch_one(&self.pool)
        .await?;

        self.row_to_user(row)
    }

    /// Update user email verification status
    pub async fn update_email_verified(&self, user_id: &UserId, email_verified: bool) -> Result<User> {
        let row = sqlx::query(
            r"
            UPDATE users
            SET email_verified = $2, updated_at = $3
            WHERE id = $1 AND deleted_at IS NULL
            RETURNING id, username, email, password_hash, signup_method, permissions, created_at, updated_at, deleted_at, email_verified
            ",
        )
        .bind(user_id.as_str())
        .bind(email_verified)
        .bind(Utc::now())
        .fetch_one(&self.pool)
        .await?;

        self.row_to_user_with_email_verified(row)
    }

    /// List users with pagination
    pub async fn list(&self, query: &UserListQuery) -> Result<(Vec<User>, i64)> {
        let offset = (query.page - 1) * query.page_size;

        // Build search condition
        let (search_condition, search_param) = if let Some(search) = &query.search {
            (
                "AND (username ILIKE $3 OR email ILIKE $3)",
                Some(format!("%{search}%")),
            )
        } else {
            ("", None)
        };

        // Get total count
        let count_query = format!(
            r"
            SELECT COUNT(*) as count
            FROM users
            WHERE deleted_at IS NULL {search_condition}
            "
        );

        let count: i64 = if let Some(ref search) = search_param {
            sqlx::query_scalar(&count_query)
                .bind(search)
                .fetch_one(&self.pool)
                .await?
        } else {
            sqlx::query_scalar(&count_query)
                .fetch_one(&self.pool)
                .await?
        };

        // Get users
        let list_query = format!(
            r"
            SELECT id, username, email, password_hash, signup_method, role, status, created_at, updated_at, deleted_at, email_verified
            FROM users
            WHERE deleted_at IS NULL {search_condition}
            ORDER BY created_at DESC
            LIMIT $1 OFFSET $2
            "
        );

        let rows = if let Some(ref search) = search_param {
            sqlx::query(&list_query)
                .bind(query.page_size)
                .bind(offset)
                .bind(search)
                .fetch_all(&self.pool)
                .await?
        } else {
            sqlx::query(&list_query)
                .bind(query.page_size)
                .bind(offset)
                .fetch_all(&self.pool)
                .await?
        };

        let users: Result<Vec<User>> = rows.into_iter().map(|row| self.row_to_user(row)).collect();

        Ok((users?, count))
    }

    /// Check if username exists
    pub async fn username_exists(&self, username: &str) -> Result<bool> {
        let count: i64 = sqlx::query_scalar(
            r"
            SELECT COUNT(*) as count
            FROM users
            WHERE username = $1 AND deleted_at IS NULL
            ",
        )
        .bind(username)
        .fetch_one(&self.pool)
        .await?;

        Ok(count > 0)
    }

    /// Check if email exists
    pub async fn email_exists(&self, email: &str) -> Result<bool> {
        let count: i64 = sqlx::query_scalar(
            r"
            SELECT COUNT(*) as count
            FROM users
            WHERE email = $1 AND deleted_at IS NULL
            ",
        )
        .bind(email)
        .fetch_one(&self.pool)
        .await?;

        Ok(count > 0)
    }

    /// Convert database row to User model
    fn row_to_user(&self, row: PgRow) -> Result<User> {
        let signup_method_str: Option<String> = row.try_get("signup_method")?;
        let signup_method = signup_method_str.map(|s| SignupMethod::from_str_name(&s));

        // Try to get email_verified, default to false if column doesn't exist
        let email_verified = row.try_get::<bool, _>("email_verified").unwrap_or_default();

        let role_str: String = row.try_get("role")?;
        let role = UserRole::from_str(&role_str)
            .map_err(|_| Error::InvalidInput(format!("Invalid role: {role_str}")))?;

        let status_str: String = row.try_get("status")?;
        let status = UserStatus::from_str(&status_str)
            .map_err(|_| Error::InvalidInput(format!("Invalid status: {status_str}")))?;

        Ok(User {
            id: UserId::from_string(row.try_get("id")?),
            username: row.try_get("username")?,
            email: row.try_get("email")?,
            password_hash: row.try_get("password_hash")?,
            signup_method,
            email_verified,
            role,
            status,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            deleted_at: row.try_get("deleted_at")?,
        })
    }

    /// Convert database row to User model (with `email_verified` explicitly included)
    fn row_to_user_with_email_verified(&self, row: PgRow) -> Result<User> {
        let signup_method_str: Option<String> = row.try_get("signup_method")?;
        let signup_method = signup_method_str.map(|s| SignupMethod::from_str_name(&s));

        let role_str: String = row.try_get("role")?;
        let role = UserRole::from_str(&role_str)
            .map_err(|_| Error::InvalidInput(format!("Invalid role: {role_str}")))?;

        let status_str: String = row.try_get("status")?;
        let status = UserStatus::from_str(&status_str)
            .map_err(|_| Error::InvalidInput(format!("Invalid status: {status_str}")))?;

        Ok(User {
            id: UserId::from_string(row.try_get("id")?),
            username: row.try_get("username")?,
            email: row.try_get("email")?,
            password_hash: row.try_get("password_hash")?,
            signup_method,
            email_verified: row.try_get("email_verified")?,
            role,
            status,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            deleted_at: row.try_get("deleted_at")?,
        })
    }
}

#[cfg(test)]
mod tests {

    // Integration tests would require a real database
    // These are placeholder tests that demonstrate the API

    #[tokio::test]
    #[ignore] // Requires database
    async fn test_create_user() {
        // This would connect to a test database
        // let pool = PgPool::connect("...").await.unwrap();
        // let repo = UserRepository::new(pool);
        // let user = User::new(...);
        // let created = repo.create(&user).await.unwrap();
        // assert_eq!(created.username, user.username);
    }
}
