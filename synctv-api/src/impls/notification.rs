//! Shared Notification Implementation
//!
//! Used by both HTTP and gRPC handlers to avoid duplicating notification logic.

use std::sync::Arc;
use synctv_core::models::id::UserId;
use synctv_core::models::notification::{
    MarkAllAsReadRequest, MarkAsReadRequest, Notification, NotificationListQuery,
    NotificationType as CoreNotificationType,
};
use synctv_core::service::UserNotificationService;
use uuid::Uuid;

/// Shared notification operations implementation.
pub struct NotificationApiImpl {
    notification_service: Arc<UserNotificationService>,
}

/// Result of listing notifications
pub struct ListNotificationsResult {
    pub notifications: Vec<Notification>,
    pub total: i64,
    pub unread_count: i64,
}

impl NotificationApiImpl {
    #[must_use]
    pub fn new(notification_service: Arc<UserNotificationService>) -> Self {
        Self {
            notification_service,
        }
    }

    /// List notifications for a user with pagination and filters.
    pub async fn list_notifications(
        &self,
        user_id: &UserId,
        page: Option<i32>,
        page_size: Option<i32>,
        is_read: Option<bool>,
        notification_type: Option<CoreNotificationType>,
    ) -> Result<ListNotificationsResult, String> {
        let query = NotificationListQuery {
            page: page.map(|p| p.max(1)),
            page_size: page_size.map(|s| s.clamp(1, 100)),
            is_read,
            notification_type,
        };

        let (notifications, total) = self
            .notification_service
            .list(user_id, query)
            .await
            .map_err(|e| format!("Failed to list notifications: {e}"))?;

        let unread_count = self
            .notification_service
            .get_unread_count(user_id)
            .await
            .map_err(|e| format!("Failed to get unread count: {e}"))?;

        Ok(ListNotificationsResult {
            notifications,
            total,
            unread_count,
        })
    }

    /// Get a single notification by ID.
    pub async fn get_notification(
        &self,
        user_id: &UserId,
        notification_id: Uuid,
    ) -> Result<Notification, String> {
        self.notification_service
            .get(user_id, notification_id)
            .await
            .map_err(|e| {
                if e.to_string().contains("not found") {
                    "Notification not found".to_string()
                } else {
                    format!("Failed to get notification: {e}")
                }
            })
    }

    /// Mark specific notifications as read.
    pub async fn mark_as_read(
        &self,
        user_id: &UserId,
        notification_ids: Vec<Uuid>,
    ) -> Result<(), String> {
        self.notification_service
            .mark_as_read(user_id, MarkAsReadRequest { notification_ids })
            .await
            .map(|_| ())
            .map_err(|e| format!("Failed to mark notifications as read: {e}"))
    }

    /// Mark all notifications as read, optionally before a timestamp.
    pub async fn mark_all_as_read(
        &self,
        user_id: &UserId,
        before: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<(), String> {
        self.notification_service
            .mark_all_as_read(user_id, MarkAllAsReadRequest { before })
            .await
            .map(|_| ())
            .map_err(|e| format!("Failed to mark all notifications as read: {e}"))
    }

    /// Delete a specific notification.
    pub async fn delete_notification(
        &self,
        user_id: &UserId,
        notification_id: Uuid,
    ) -> Result<(), String> {
        self.notification_service
            .delete(user_id, notification_id)
            .await
            .map_err(|e| {
                if e.to_string().contains("not found") {
                    "Notification not found".to_string()
                } else {
                    format!("Failed to delete notification: {e}")
                }
            })
    }

    /// Delete all read notifications for a user.
    pub async fn delete_all_read(&self, user_id: &UserId) -> Result<(), String> {
        self.notification_service
            .delete_all_read(user_id)
            .await
            .map(|_| ())
            .map_err(|e| format!("Failed to delete all read notifications: {e}"))
    }
}
