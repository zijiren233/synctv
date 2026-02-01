//! OAuth2/OIDC provider repository
//!
//! This repository manages OAuth2 provider mappings (NOT TOKENS).

use sqlx::{PgPool, FromRow};
use crate::{
    models::{oauth2_client::*, UserId},
    Result,
};

/// OAuth2/OIDC provider repository
///
/// Manages mappings between OAuth2 providers and local users.
/// Tokens are NOT stored - only provider identity information.
#[derive(Clone)]
pub struct UserOAuthProviderRepository {
    pool: PgPool,
}

impl UserOAuthProviderRepository {
    /// Create new repository
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Insert or update OAuth2 provider mapping
    pub async fn upsert(
        &self,
        user_id: &UserId,
        provider: &OAuth2Provider,
        provider_user_id: &str,
        user_info: &OAuth2UserInfo,
    ) -> Result<()> {
        let id = nanoid::nanoid!(12);

        sqlx::query(
            r#"
            INSERT INTO oauth2_clients (id, provider, provider_user_id, user_id, username, email, avatar_url)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (provider, provider_user_id)
            DO UPDATE SET
                user_id = EXCLUDED.user_id,
                username = EXCLUDED.username,
                email = EXCLUDED.email,
                avatar_url = EXCLUDED.avatar_url,
                updated_at = CURRENT_TIMESTAMP
            "#
        )
        .bind(&id)
        .bind(provider.as_str())
        .bind(provider_user_id)
        .bind(user_id.as_str())
        .bind(&user_info.username)
        .bind(&user_info.email)
        .bind(&user_info.avatar)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Find user by OAuth2 provider and provider user ID
    pub async fn find_by_provider(
        &self,
        provider: &OAuth2Provider,
        provider_user_id: &str,
    ) -> Result<Option<UserOAuthProviderMapping>> {
        let row = sqlx::query_as::<_, OAuth2ClientRow>(
            "SELECT * FROM oauth2_clients WHERE provider = $1 AND provider_user_id = $2"
        )
        .bind(provider.as_str())
        .bind(provider_user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into()))
    }

    /// Find all OAuth2 providers for a user
    pub async fn find_by_user(&self, user_id: &UserId) -> Result<Vec<UserOAuthProviderMapping>> {
        let rows = sqlx::query_as::<_, OAuth2ClientRow>(
            "SELECT * FROM oauth2_clients WHERE user_id = $1"
        )
        .bind(user_id.as_str())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// Delete OAuth2 provider mapping
    pub async fn delete(
        &self,
        user_id: &UserId,
        provider: &OAuth2Provider,
        provider_user_id: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            "DELETE FROM oauth2_clients WHERE user_id = $1 AND provider = $2 AND provider_user_id = $3"
        )
        .bind(user_id.as_str())
        .bind(provider.as_str())
        .bind(provider_user_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}

/// Row representation for SQL queries (user_id as String)
#[derive(FromRow)]
struct OAuth2ClientRow {
    pub id: String,
    pub provider: String,
    pub provider_user_id: String,
    pub user_id: String,
    pub username: String,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<OAuth2ClientRow> for UserOAuthProviderMapping {
    fn from(row: OAuth2ClientRow) -> Self {
        Self {
            id: row.id,
            provider: row.provider,
            provider_user_id: row.provider_user_id,
            user_id: UserId(row.user_id),
            username: row.username,
            email: row.email,
            avatar_url: row.avatar_url,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}
