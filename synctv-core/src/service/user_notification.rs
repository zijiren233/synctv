//! User notification service
//!
//! Manages user notifications for room invitations, system announcements, and room events
//! These are database-backed notifications that persist until read/deleted

use uuid::Uuid;

use crate::{
    models::{
        id::UserId,
        notification::{
            CreateNotificationRequest, MarkAllAsReadRequest, MarkAsReadRequest, Notification,
            NotificationListQuery, NotificationType,
        },
    },
    repository::NotificationRepository,
    Error,
    Result,
};

/// User notification service
#[derive(Clone, Debug)]
pub struct UserNotificationService {
    repository: NotificationRepository,
}

impl UserNotificationService {
    #[must_use] 
    pub const fn new(repository: NotificationRepository) -> Self {
        Self { repository }
    }

    /// Create a new notification
    pub async fn create(&self, req: CreateNotificationRequest) -> Result<Notification> {
        self.repository.create(&req).await
    }

    /// Create a room invitation notification
    pub async fn create_room_invitation(
        &self,
        user_id: UserId,
        room_id: String,
        room_name: String,
        inviter_name: String,
    ) -> Result<Notification> {
        let data = serde_json::json!({
            "room_id": room_id,
            "room_name": room_name,
            "inviter_name": inviter_name,
        });

        let req = CreateNotificationRequest {
            user_id,
            notification_type: NotificationType::RoomInvitation,
            title: format!("Room Invitation: {room_name}"),
            content: format!("{inviter_name} invited you to join the room \"{room_name}\""),
            data,
        };

        self.create(req).await
    }

    /// Create a system announcement
    pub async fn create_system_announcement(
        &self,
        user_id: UserId,
        title: String,
        content: String,
        data: Option<serde_json::Value>,
    ) -> Result<Notification> {
        let req = CreateNotificationRequest {
            user_id,
            notification_type: NotificationType::SystemAnnouncement,
            title,
            content,
            data: data.unwrap_or_default(),
        };

        self.create(req).await
    }

    /// Create a room event notification
    pub async fn create_room_event(
        &self,
        user_id: UserId,
        room_id: String,
        room_name: String,
        event: String,
    ) -> Result<Notification> {
        let data = serde_json::json!({
            "room_id": room_id,
            "room_name": room_name,
            "event": event,
        });

        let req = CreateNotificationRequest {
            user_id,
            notification_type: NotificationType::RoomEvent,
            title: format!("Room Event: {room_name}"),
            content: event,
            data,
        };

        self.create(req).await
    }

    /// Get notification by ID
    pub async fn get(&self, user_id: &UserId, notification_id: Uuid) -> Result<Notification> {
        self.repository
            .get_by_id(notification_id)
            .await?
            .filter(|n| n.user_id == *user_id)
            .ok_or_else(|| Error::NotFound("Notification not found".to_string()))
    }

    /// List notifications for a user
    pub async fn list(
        &self,
        user_id: &UserId,
        query: NotificationListQuery,
    ) -> Result<(Vec<Notification>, i64)> {
        let notifications = self.repository.list_by_user(user_id, &query).await?;

        let total = self
            .repository
            .count_by_user(user_id, query.is_read, query.notification_type.as_ref())
            .await?;

        Ok((notifications, total))
    }

    /// Get unread count for a user
    pub async fn get_unread_count(&self, user_id: &UserId) -> Result<i64> {
        self.repository.count_unread(user_id).await
    }

    /// Mark notifications as read
    pub async fn mark_as_read(
        &self,
        user_id: &UserId,
        req: MarkAsReadRequest,
    ) -> Result<usize> {
        let affected = self.repository.mark_as_read(user_id, &req.notification_ids).await?;
        Ok(affected as usize)
    }

    /// Mark all notifications as read
    pub async fn mark_all_as_read(
        &self,
        user_id: &UserId,
        req: MarkAllAsReadRequest,
    ) -> Result<usize> {
        let affected = self
            .repository
            .mark_all_as_read(user_id, req.before)
            .await?;
        Ok(affected as usize)
    }

    /// Delete a notification
    pub async fn delete(&self, user_id: &UserId, notification_id: Uuid) -> Result<()> {
        self.repository.delete(user_id, notification_id).await
    }

    /// Delete all read notifications
    pub async fn delete_all_read(&self, user_id: &UserId) -> Result<usize> {
        let affected = self.repository.delete_all_read(user_id).await?;
        Ok(affected as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_type_from_str() {
        assert_eq!(
            "room_invitation".parse::<NotificationType>().unwrap(),
            NotificationType::RoomInvitation
        );
    }
}
