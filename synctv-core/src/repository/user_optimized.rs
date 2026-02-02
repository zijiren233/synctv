//! Optimized user repository with advanced query building
//!
//! Demonstrates complex query scenarios using BoolExpr

use sqlx::{PgPool, postgres::PgRow, Row};
use chrono::{DateTime, Utc};

use crate::{
    models::{PermissionBits, SignupMethod, User, UserId, UserListQuery},
    repository::{BoolExpr, Column, FilterBuilder},
    Error, Result,
};

/// Advanced user query builder with boolean filters
pub struct UserFilterBuilder {
    builder: FilterBuilder,
    custom_filters: Vec<BoolExpr>,
}

impl Default for UserFilterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl UserFilterBuilder {
    /// Create a new user filter builder
    pub fn new() -> Self {
        Self {
            builder: FilterBuilder::new(),
            custom_filters: Vec::new(),
        }
    }

    /// Filter by exact username
    pub fn username(mut self, username: impl Into<String>) -> Self {
        self.builder = self.builder.eq("u.username", username.into());
        self
    }

    /// Filter by username pattern (case-insensitive)
    pub fn username_like(mut self, pattern: impl Into<String>) -> Self {
        self.builder = self.builder.ilike("u.username", pattern.into());
        self
    }

    /// Filter by exact email
    pub fn email(mut self, email: impl Into<String>) -> Self {
        self.builder = self.builder.eq("u.email", email.into());
        self
    }

    /// Filter by email pattern (case-insensitive)
    pub fn email_like(mut self, pattern: impl Into<String>) -> Self {
        self.builder = self.builder.ilike("u.email", pattern.into());
        self
    }

    /// Filter by signup method
    pub fn signup_method(mut self, method: SignupMethod) -> Self {
        let method_str = match method {
            SignupMethod::Email => "email",
            SignupMethod::OAuth2 => "oauth2",
        };
        self.builder = self.builder.eq("u.signup_method", method_str);
        self
    }

    /// Filter by minimum permissions (bitwise AND)
    pub fn has_permissions(mut self, permissions: PermissionBits) -> Self {
        // Use raw SQL for bitwise operation
        self.custom_filters.push(BoolExpr::raw(format!(
            "(u.permissions & {}) > 0",
            permissions.0
        )));
        self
    }

    /// Filter by exact permissions match
    pub fn permissions_eq(mut self, permissions: PermissionBits) -> Self {
        self.builder = self.builder.eq("u.permissions", permissions.0);
        self
    }

    /// Filter by minimum permission level
    pub fn permissions_gte(mut self, permissions: PermissionBits) -> Self {
        self.custom_filters.push(BoolExpr::raw(format!(
            "(u.permissions & {}) = {}",
            permissions.0,
            permissions.0
        )));
        self
    }

    /// Filter by creation date range
    pub fn created_between(mut self, start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        self.builder = self.builder.between("u.created_at", start, end);
        self
    }

    /// Filter by users created after a date
    pub fn created_after(mut self, date: DateTime<Utc>) -> Self {
        self.builder = self.builder.ge("u.created_at", date);
        self
    }

    /// Filter by users created before a date
    pub fn created_before(mut self, date: DateTime<Utc>) -> Self {
        self.builder = self.builder.le("u.created_at", date);
        self
    }

    /// Filter by email verification status
    pub fn email_verified(mut self, verified: bool) -> Self {
        self.builder = self.builder.eq("u.email_verified", verified);
        self
    }

    /// Include deleted users (soft delete)
    pub fn include_deleted(mut self) -> Self {
        // Default is to exclude deleted users, so we don't add a filter for deleted_at
        self
    }

    /// Exclude deleted users (default)
    pub fn exclude_deleted(mut self) -> Self {
        self.builder = self.builder.is_null("u.deleted_at");
        self
    }

    /// Add a custom boolean expression
    pub fn custom(mut self, expr: BoolExpr) -> Self {
        self.custom_filters.push(expr);
        self
    }

    /// Build the final boolean filter
    pub fn build(self) -> BoolExpr {
        let base_filter = self.builder.build();

        if self.custom_filters.is_empty() {
            base_filter
        } else {
            // Combine base filter with custom filters
            let mut all_filters = vec![base_filter];
            all_filters.extend(self.custom_filters);
            BoolExpr::and(all_filters)
        }
    }
}

/// User repository with optimized query building
#[derive(Clone)]
pub struct UserRepository {
    pool: PgPool,
}

impl UserRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get the database pool
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Get user by ID
    pub async fn get_by_id(&self, user_id: &UserId) -> Result<Option<User>> {
        let row = sqlx::query(
            r#"
            SELECT id, username, email, password_hash, signup_method, permissions,
                   created_at, updated_at, deleted_at, email_verified
            FROM users u
            WHERE u.id = $1 AND u.deleted_at IS NULL
            "#
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
            r#"
            SELECT id, username, email, password_hash, signup_method, permissions,
                   created_at, updated_at, deleted_at, email_verified
            FROM users u
            WHERE u.username = $1 AND u.deleted_at IS NULL
            "#
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
            r#"
            SELECT id, username, email, password_hash, signup_method, permissions,
                   created_at, updated_at, deleted_at, email_verified
            FROM users u
            WHERE u.email = $1 AND u.deleted_at IS NULL
            "#
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_user(row)?)),
            None => Ok(None),
        }
    }

    /// Get multiple users by IDs (batch query with IN clause)
    pub async fn get_by_ids(&self, user_ids: &[UserId]) -> Result<Vec<User>> {
        if user_ids.is_empty() {
            return Ok(Vec::new());
        }

        let ids: Vec<&str> = user_ids.iter().map(|id| id.as_str()).collect();
        let placeholders = (1..=ids.len())
            .map(|i| format!("${}", i))
            .collect::<Vec<_>>()
            .join(", ");

        let query = format!(
            r#"
            SELECT id, username, email, password_hash, signup_method, permissions,
                   created_at, updated_at, deleted_at, email_verified
            FROM users u
            WHERE u.id IN ({}) AND u.deleted_at IS NULL
            ORDER BY u.created_at DESC
            "#,
            placeholders
        );

        let mut query_builder = sqlx::query(&query);
        for id in ids {
            query_builder = query_builder.bind(id);
        }

        let rows = query_builder.fetch_all(&self.pool).await?;
        let users: Result<Vec<User>> = rows.into_iter().map(|row| self.row_to_user(row)).collect();

        users
    }

    /// List users with advanced filtering using UserFilterBuilder
    pub async fn list_filtered(
        &self,
        filter: &UserFilterBuilder,
        page: i64,
        page_size: i64,
    ) -> Result<(Vec<User>, i64)> {
        let bool_expr = filter.build();
        let offset = (page - 1) * page_size;

        // Get total count
        let count_query = format!(
            "SELECT COUNT(*) as count FROM users u WHERE {}",
            bool_expr.to_sql()
        );
        let count: i64 = sqlx::query_scalar(&count_query)
            .fetch_one(&self.pool)
            .await?;

        // Get users
        let list_query = format!(
            r#"
            SELECT u.id, u.username, u.email, u.password_hash, u.signup_method, u.permissions,
                   u.created_at, u.updated_at, u.deleted_at, u.email_verified
            FROM users u
            WHERE {}
            ORDER BY u.created_at DESC
            LIMIT $1 OFFSET $2
            "#,
            bool_expr.to_sql()
        );

        let rows = sqlx::query(&list_query)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;

        let users: Result<Vec<User>> = rows.into_iter().map(|row| self.row_to_user(row)).collect();

        Ok((users?, count))
    }

    /// Legacy list method (converts from UserListQuery)
    pub async fn list(&self, query: &UserListQuery) -> Result<(Vec<User>, i64)> {
        let mut filter_builder = UserFilterBuilder::new().exclude_deleted();

        // Apply filters from query
        if let Some(username_pattern) = &query.username_pattern {
            filter_builder = filter_builder.username_like(format!("%{}%", username_pattern));
        }

        if let Some(email_pattern) = &query.email_pattern {
            filter_builder = filter_builder.email_like(format!("%{}%", email_pattern));
        }

        if let Some(email_verified) = query.email_verified {
            filter_builder = filter_builder.email_verified(*email_verified);
        }

        if let Some(min_permissions) = query.min_permissions {
            filter_builder = filter_builder.permissions_gte(min_permissions);
        }

        self.list_filtered(&filter_builder, query.page, query.page_size)
            .await
    }

    /// Find users with complex boolean filter
    pub async fn find(
        &self,
        filter: &BoolExpr,
        page: i64,
        page_size: i64,
    ) -> Result<(Vec<User>, i64)> {
        let offset = (page - 1) * page_size;

        // Always exclude deleted users
        let filter = BoolExpr::and(vec![
            filter.clone(),
            BoolExpr::is_null("u.deleted_at"),
        ]);

        // Get total count
        let count_query = format!(
            "SELECT COUNT(*) as count FROM users u WHERE {}",
            filter.to_sql()
        );
        let count: i64 = sqlx::query_scalar(&count_query)
            .fetch_one(&self.pool)
            .await?;

        // Get users
        let list_query = format!(
            r#"
            SELECT u.id, u.username, u.email, u.password_hash, u.signup_method, u.permissions,
                   u.created_at, u.updated_at, u.deleted_at, u.email_verified
            FROM users u
            WHERE {}
            ORDER BY u.created_at DESC
            LIMIT $1 OFFSET $2
            "#,
            filter.to_sql()
        );

        let rows = sqlx::query(&list_query)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;

        let users: Result<Vec<User>> = rows.into_iter().map(|row| self.row_to_user(row)).collect();

        Ok((users?, count))
    }

    /// Count users matching a filter
    pub async fn count_filtered(&self, filter: &BoolExpr) -> Result<i64> {
        let filter = BoolExpr::and(vec![
            filter.clone(),
            BoolExpr::is_null("u.deleted_at"),
        ]);

        let count_query = format!(
            "SELECT COUNT(*) as count FROM users u WHERE {}",
            filter.to_sql()
        );
        let count: i64 = sqlx::query_scalar(&count_query)
            .fetch_one(&self.pool)
            .await?;

        Ok(count)
    }

    /// Check if username exists
    pub async fn username_exists(&self, username: &str) -> Result<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) as count
             FROM users u
             WHERE u.username = $1 AND u.deleted_at IS NULL"
        )
        .bind(username)
        .fetch_one(&self.pool)
        .await?;

        Ok(count > 0)
    }

    /// Check if email exists
    pub async fn email_exists(&self, email: &str) -> Result<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) as count
             FROM users u
             WHERE u.email = $1 AND u.deleted_at IS NULL"
        )
        .bind(email)
        .fetch_one(&self.pool)
        .await?;

        Ok(count > 0)
    }

    /// Convert database row to User model
    fn row_to_user(&self, row: PgRow) -> Result<User> {
        let signup_method: Option<String> = row.try_get("signup_method")?;
        let signup_method = signup_method.map(|s| match s.as_str() {
            "email" => SignupMethod::Email,
            "oauth2" => SignupMethod::OAuth2,
            _ => SignupMethod::Email,
        });

        Ok(User {
            id: UserId::from_string(row.try_get("id")?),
            username: row.try_get("username")?,
            email: row.try_get("email")?,
            password_hash: row.try_get("password_hash")?,
            signup_method,
            permissions: PermissionBits(row.try_get::<i32>("permissions")?),
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            deleted_at: row.try_get("deleted_at")?,
            email_verified: row.try_get("email_verified")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SignupMethod;

    #[tokio::test]
    fn test_user_filter_builder_simple() {
        let filter = UserFilterBuilder::new()
            .username("alice")
            .email_verified(true)
            .exclude_deleted()
            .build();

        let sql = filter.to_sql();
        assert!(sql.contains("u.username = 'alice'"));
        assert!(sql.contains("u.email_verified = TRUE"));
        assert!(sql.contains("u.deleted_at IS NULL"));
    }

    #[tokio::test]
    fn test_user_filter_builder_complex() {
        let filter = UserFilterBuilder::new()
            .username_like("%admin%")
            .email_like("%@example.com")
            .signup_method(SignupMethod::Email)
            .has_permissions(PermissionBits(0b1111))
            .created_after(
                Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
            )
            .build();

        let sql = filter.to_sql();
        println!("Generated SQL: {}", sql);

        assert!(sql.contains("u.username ILIKE '%admin%'"));
        assert!(sql.contains("u.email ILIKE '%@example.com'"));
        assert!(sql.contains("u.signup_method = 'email'"));
        assert!(sql.contains("(u.permissions & 15) > 0"));
        assert!(sql.contains("u.created_at >"));
    }

    #[tokio::test]
    fn test_complex_boolean_expressions() {
        // Test OR conditions
        let filter = BoolExpr::and(vec![
            BoolExpr::eq("u.status", "active"),
            BoolExpr::or(vec![
                BoolExpr::eq("u.role", "admin"),
                BoolExpr::eq("u.role", "moderator"),
            ]),
            BoolExpr::not(BoolExpr::eq("u.deleted", true)),
        ]);

        let sql = filter.to_sql();
        println!("Complex filter SQL: {}", sql);

        assert!(sql.contains("AND"));
        assert!(sql.contains("OR"));
        assert!(sql.contains("NOT"));
    }

    #[tokio::test]
    fn test_batch_query_with_in() {
        // Test IN clause generation
        let filter = BoolExpr::and(vec![
            BoolExpr::in_list(
                "u.id",
                vec![
                    Value::String("user1".to_string()),
                    Value::String("user2".to_string()),
                    Value::String("user3".to_string()),
                ],
            ),
            BoolExpr::is_null("u.deleted_at"),
        ]);

        let sql = filter.to_sql();
        assert!(sql.contains("u.id IN"));
        assert!(sql.contains("user1"));
        assert!(sql.contains("user2"));
        assert!(sql.contains("user3"));
    }

    #[tokio::test]
    fn test_qualified_columns() {
        let filter = FilterBuilder::new()
            .eq(Column::qualified("users", "username"), "test")
            .build();

        let sql = filter.to_sql();
        assert!(sql.contains("users.username"));
    }
}
