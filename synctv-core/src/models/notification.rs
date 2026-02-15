//! Notification models
//!
//! User notifications for room invitations, system announcements, and room events

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::id::UserId;

/// Notification type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NotificationType {
    /// Room invitation from another user
    RoomInvitation,
    /// System announcement from administrators
    SystemAnnouncement,
    /// Room event (e.g., user joined, media added)
    RoomEvent,
    /// Password reset notification
    PasswordReset,
    /// Email verification reminder
    EmailVerification,
}

impl std::fmt::Display for NotificationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RoomInvitation => write!(f, "room_invitation"),
            Self::SystemAnnouncement => write!(f, "system_announcement"),
            Self::RoomEvent => write!(f, "room_event"),
            Self::PasswordReset => write!(f, "password_reset"),
            Self::EmailVerification => write!(f, "email_verification"),
        }
    }
}

impl std::str::FromStr for NotificationType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "room_invitation" => Ok(Self::RoomInvitation),
            "system_announcement" => Ok(Self::SystemAnnouncement),
            "room_event" => Ok(Self::RoomEvent),
            "password_reset" => Ok(Self::PasswordReset),
            "email_verification" => Ok(Self::EmailVerification),
            _ => Err(anyhow::anyhow!("Invalid notification type: {s}")),
        }
    }
}

// Database mapping: NotificationType <-> TEXT
impl sqlx::Type<sqlx::Postgres> for NotificationType {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl sqlx::Encode<'_, sqlx::Postgres> for NotificationType {
    fn encode_by_ref(&self, buf: &mut sqlx::postgres::PgArgumentBuffer) -> Result<sqlx::encode::IsNull, Box<dyn std::error::Error + Send + Sync>> {
        let s = self.to_string();
        <String as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&s, buf)
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for NotificationType {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        s.parse().map_err(|e: anyhow::Error| e.into())
    }
}

/// Notification model
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Notification {
    pub id: Uuid,
    pub user_id: UserId,
    #[sqlx(rename = "type")]
    pub notification_type: NotificationType,
    pub title: String,
    pub content: String,
    pub data: serde_json::Value,
    pub is_read: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Create notification request
#[derive(Debug, Deserialize)]
pub struct CreateNotificationRequest {
    pub user_id: UserId,
    pub notification_type: NotificationType,
    pub title: String,
    pub content: String,
    #[serde(default = "default_empty_data")]
    pub data: serde_json::Value,
}

fn default_empty_data() -> serde_json::Value {
    serde_json::json!({})
}

/// List notifications query parameters
#[derive(Debug, Deserialize)]
pub struct NotificationListQuery {
    pub pagination: super::pagination::PageParams,
    pub is_read: Option<bool>,
    pub notification_type: Option<NotificationType>,
}

/// Mark notification as read request
#[derive(Debug, Deserialize)]
pub struct MarkAsReadRequest {
    pub notification_ids: Vec<Uuid>,
}

/// Mark all notifications as read request
#[derive(Debug, Deserialize)]
pub struct MarkAllAsReadRequest {
    pub before: Option<chrono::DateTime<chrono::Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_type_display() {
        assert_eq!(NotificationType::RoomInvitation.to_string(), "room_invitation");
        assert_eq!(NotificationType::SystemAnnouncement.to_string(), "system_announcement");
    }

    #[test]
    fn test_notification_type_from_str() {
        assert_eq!(
            "room_invitation".parse::<NotificationType>().unwrap(),
            NotificationType::RoomInvitation
        );
        assert_eq!(
            "system_announcement".parse::<NotificationType>().unwrap(),
            NotificationType::SystemAnnouncement
        );
    }
}
