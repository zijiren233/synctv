//! Email token repository for database operations

use chrono::Utc;
use sqlx::{postgres::PgRow, PgPool, Row};
use crate::{
    models::UserId,
    service::email_token::EmailTokenType,
    Error, Result,
};

/// Email token record
#[derive(Debug, Clone)]
pub struct EmailToken {
    pub id: String,
    pub token: String,
    pub user_id: UserId,
    pub token_type: String,
    pub expires_at: chrono::DateTime<Utc>,
    pub used_at: Option<chrono::DateTime<Utc>>,
    pub created_at: chrono::DateTime<Utc>,
}

/// Email token repository
#[derive(Clone)]
pub struct EmailTokenRepository {
    pool: PgPool,
}

impl EmailTokenRepository {
    #[must_use] 
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a new token
    pub async fn create(
        &self,
        token: &str,
        user_id: &UserId,
        token_type: EmailTokenType,
        expires_at: chrono::DateTime<Utc>,
    ) -> Result<EmailToken> {
        let row = sqlx::query(
            r"
            INSERT INTO email_tokens (token, user_id, token_type, expires_at, created_at)
            VALUES ($1, $2, $3, $4, CURRENT_TIMESTAMP)
            RETURNING id, token, user_id, token_type, expires_at, used_at, created_at
            ",
        )
        .bind(token)
        .bind(user_id.as_str())
        .bind(token_type.as_str())
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await
        .map_err(Error::Database)?;

        self.row_to_token(row)
    }

    /// Get token by token string
    pub async fn get(&self, token: &str) -> Result<Option<EmailToken>> {
        let row = sqlx::query(
            r"
            SELECT id, token, user_id, token_type, expires_at, used_at, created_at
            FROM email_tokens
            WHERE token = $1
            ",
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => Ok(Some(self.row_to_token(r)?)),
            None => Ok(None),
        }
    }

    /// Mark token as used
    pub async fn mark_as_used(&self, token: &str) -> Result<EmailToken> {
        let row = sqlx::query(
            r"
            UPDATE email_tokens
            SET used_at = CURRENT_TIMESTAMP
            WHERE token = $1
            RETURNING id, token, user_id, token_type, expires_at, used_at, created_at
            ",
        )
        .bind(token)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => {
                Error::InvalidInput("Token not found".to_string())
            }
            _ => Error::Database(e),
        })?;

        self.row_to_token(row)
    }

    /// Atomically validate and consume a token.
    ///
    /// In a single UPDATE, checks that the token exists, matches the expected type,
    /// has not been used, and has not expired. If all conditions are met, marks it
    /// as used and returns the record. Returns `None` if any condition fails.
    pub async fn validate_and_consume(
        &self,
        token: &str,
        token_type: EmailTokenType,
    ) -> Result<Option<EmailToken>> {
        let row = sqlx::query(
            r"
            UPDATE email_tokens
            SET used_at = CURRENT_TIMESTAMP
            WHERE token = $1
              AND token_type = $2
              AND used_at IS NULL
              AND expires_at > CURRENT_TIMESTAMP
            RETURNING id, token, user_id, token_type, expires_at, used_at, created_at
            ",
        )
        .bind(token)
        .bind(token_type.as_str())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => Ok(Some(self.row_to_token(r)?)),
            None => Ok(None),
        }
    }

    /// Delete all tokens of a specific type for a user
    pub async fn delete_user_tokens(
        &self,
        user_id: &UserId,
        token_type: EmailTokenType,
    ) -> Result<u64> {
        let result = sqlx::query(
            r"
            DELETE FROM email_tokens
            WHERE user_id = $1 AND token_type = $2 AND used_at IS NULL
            ",
        )
        .bind(user_id.as_str())
        .bind(token_type.as_str())
        .execute(&self.pool)
        .await
        .map_err(Error::Database)?;

        Ok(result.rows_affected())
    }

    /// Cleanup expired tokens
    pub async fn cleanup_expired(&self) -> Result<usize> {
        let result = sqlx::query(
            r"
            DELETE FROM email_tokens
            WHERE expires_at < CURRENT_TIMESTAMP
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(Error::Database)?;

        Ok(result.rows_affected() as usize)
    }

    fn row_to_token(&self, row: PgRow) -> Result<EmailToken> {
        Ok(EmailToken {
            id: row.try_get("id")?,
            token: row.try_get("token")?,
            user_id: UserId::from_string(row.try_get("user_id")?),
            token_type: row.try_get("token_type")?,
            expires_at: row.try_get("expires_at")?,
            used_at: row.try_get("used_at")?,
            created_at: row.try_get("created_at")?,
        })
    }
}

#[cfg(test)]
mod tests {
}
