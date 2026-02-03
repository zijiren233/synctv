use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::{
    models::{
        id::UserId,
        notification::{CreateNotificationRequest, Notification, NotificationListQuery, NotificationType},
    },
    Error,
    Result,
};

/// Notification repository for database operations
#[derive(Clone, Debug)]
pub struct NotificationRepository {
    pool: PgPool,
}

impl NotificationRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a new notification
    pub async fn create(&self, req: &CreateNotificationRequest) -> Result<Notification> {
        let id = Uuid::new_v4();
        let now = Utc::now();

        let row = sqlx::query(
            r#"
            INSERT INTO notifications (id, user_id, type, title, content, data, is_read, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING id, user_id, type, title, content, data, is_read, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(req.user_id.as_str())
        .bind(req.notification_type.to_string())
        .bind(&req.title)
        .bind(&req.content)
        .bind(&req.data)
        .bind(false)
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;

        self.row_to_notification(row)
    }

    /// Get notification by ID
    pub async fn get_by_id(&self, notification_id: Uuid) -> Result<Option<Notification>> {
        let row = sqlx::query(
            r#"
            SELECT id, user_id, type, title, content, data, is_read, created_at, updated_at
            FROM notifications
            WHERE id = $1
            "#,
        )
        .bind(notification_id)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_notification(row)?)),
            None => Ok(None),
        }
    }

    /// List notifications for a user with pagination and filters
    pub async fn list_by_user(
        &self,
        user_id: &UserId,
        query: &NotificationListQuery,
    ) -> Result<Vec<Notification>> {
        let page = query.page.unwrap_or(1).max(1);
        let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
        let offset = (page - 1) * page_size;

        let rows = if let Some(notification_type) = &query.notification_type {
            if let Some(is_read) = query.is_read {
                sqlx::query(
                    r#"
                    SELECT id, user_id, type, title, content, data, is_read, created_at, updated_at
                    FROM notifications
                    WHERE user_id = $1 AND type = $2 AND is_read = $3
                    ORDER BY created_at DESC
                    LIMIT $4 OFFSET $5
                    "#,
                )
                .bind(user_id.as_str())
                .bind(notification_type.to_string())
                .bind(is_read)
                .bind(page_size)
                .bind(offset as i64)
                .fetch_all(&self.pool)
                .await?
            } else {
                sqlx::query(
                    r#"
                    SELECT id, user_id, type, title, content, data, is_read, created_at, updated_at
                    FROM notifications
                    WHERE user_id = $1 AND type = $2
                    ORDER BY created_at DESC
                    LIMIT $3 OFFSET $4
                    "#,
                )
                .bind(user_id.as_str())
                .bind(notification_type.to_string())
                .bind(page_size)
                .bind(offset as i64)
                .fetch_all(&self.pool)
                .await?
            }
        } else if let Some(is_read) = query.is_read {
            sqlx::query(
                r#"
                SELECT id, user_id, type, title, content, data, is_read, created_at, updated_at
                FROM notifications
                WHERE user_id = $1 AND is_read = $2
                ORDER BY created_at DESC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(user_id.as_str())
            .bind(is_read)
            .bind(page_size)
            .bind(offset as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"
                SELECT id, user_id, type, title, content, data, is_read, created_at, updated_at
                FROM notifications
                WHERE user_id = $1
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(user_id.as_str())
            .bind(page_size)
            .bind(offset as i64)
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter()
            .map(|row| self.row_to_notification(row))
            .collect()
    }

    /// Count notifications for a user (for pagination)
    pub async fn count_by_user(
        &self,
        user_id: &UserId,
        is_read: Option<bool>,
        notification_type: Option<&NotificationType>,
    ) -> Result<i64> {
        let count: i64 = if let Some(notification_type) = notification_type {
            if let Some(is_read) = is_read {
                sqlx::query_scalar(
                    r#"
                    SELECT COUNT(*)
                    FROM notifications
                    WHERE user_id = $1 AND type = $2 AND is_read = $3
                    "#,
                )
                .bind(user_id.as_str())
                .bind(notification_type.to_string())
                .bind(is_read)
                .fetch_one(&self.pool)
                .await?
            } else {
                sqlx::query_scalar(
                    r#"
                    SELECT COUNT(*)
                    FROM notifications
                    WHERE user_id = $1 AND type = $2
                    "#,
                )
                .bind(user_id.as_str())
                .bind(notification_type.to_string())
                .fetch_one(&self.pool)
                .await?
            }
        } else if let Some(is_read) = is_read {
            sqlx::query_scalar(
                r#"
                SELECT COUNT(*)
                FROM notifications
                WHERE user_id = $1 AND is_read = $2
                "#,
            )
            .bind(user_id.as_str())
            .bind(is_read)
            .fetch_one(&self.pool)
            .await?
        } else {
            sqlx::query_scalar(
                r#"
                SELECT COUNT(*)
                FROM notifications
                WHERE user_id = $1
                "#,
            )
            .bind(user_id.as_str())
            .fetch_one(&self.pool)
            .await?
        };

        Ok(count)
    }

    /// Get unread count for a user
    pub async fn count_unread(&self, user_id: &UserId) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*)
            FROM notifications
            WHERE user_id = $1 AND is_read = FALSE
            "#,
        )
        .bind(user_id.as_str())
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }

    /// Mark notifications as read
    pub async fn mark_as_read(&self, user_id: &UserId, notification_ids: &[Uuid]) -> Result<u64> {
        if notification_ids.is_empty() {
            return Ok(0);
        }

        let result = sqlx::query(
            r#"
            UPDATE notifications
            SET is_read = TRUE, updated_at = NOW()
            WHERE user_id = $1 AND id = ANY($2)
            "#,
        )
        .bind(user_id.as_str())
        .bind(notification_ids)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    /// Mark all notifications as read before a certain time (or all if no time specified)
    pub async fn mark_all_as_read(
        &self,
        user_id: &UserId,
        before: Option<DateTime<Utc>>,
    ) -> Result<u64> {
        let result = if let Some(before_time) = before {
            sqlx::query(
                r#"
                UPDATE notifications
                SET is_read = TRUE, updated_at = NOW()
                WHERE user_id = $1 AND is_read = FALSE AND created_at <= $2
                "#,
            )
            .bind(user_id.as_str())
            .bind(before_time)
            .execute(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"
                UPDATE notifications
                SET is_read = TRUE, updated_at = NOW()
                WHERE user_id = $1 AND is_read = FALSE
                "#,
            )
            .bind(user_id.as_str())
            .execute(&self.pool)
            .await?
        };

        Ok(result.rows_affected())
    }

    /// Delete a notification
    pub async fn delete(&self, user_id: &UserId, notification_id: Uuid) -> Result<()> {
        let result = sqlx::query(
            r#"
            DELETE FROM notifications
            WHERE user_id = $1 AND id = $2
            "#,
        )
        .bind(user_id.as_str())
        .bind(notification_id)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(Error::NotFound("Notification not found".to_string()));
        }

        Ok(())
    }

    /// Delete all read notifications for a user
    pub async fn delete_all_read(&self, user_id: &UserId) -> Result<u64> {
        let result = sqlx::query(
            r#"
            DELETE FROM notifications
            WHERE user_id = $1 AND is_read = TRUE
            "#,
        )
        .bind(user_id.as_str())
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    /// Helper method to convert database row to Notification
    fn row_to_notification(&self, row: sqlx::postgres::PgRow) -> Result<Notification> {
        let type_str: String = row.try_get("type")?;
        let notification_type = type_str.parse()
            .map_err(|e| Error::Internal(format!("Invalid notification type: {}", e)))?;

        Ok(Notification {
            id: row.try_get("id")?,
            user_id: UserId::from_string(row.try_get("user_id")?),
            notification_type,
            title: row.try_get("title")?,
            content: row.try_get("content")?,
            data: row.try_get("data")?,
            is_read: row.try_get("is_read")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}
