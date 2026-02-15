//! gRPC NotificationService implementation
//!
//! Thin wrapper that delegates to `UserNotificationService` from synctv-core,
//! converting between proto types and domain types.

use std::sync::Arc;
use tonic::{Request, Response, Status};
use uuid::Uuid;

use synctv_core::models::id::UserId;
use synctv_core::models::notification::{
    NotificationListQuery, NotificationType as CoreNotificationType,
};
use synctv_core::service::UserNotificationService;

use crate::proto::client::{
    notification_service_server::NotificationService, DeleteAllReadRequest, DeleteAllReadResponse,
    DeleteNotificationRequest, DeleteNotificationResponse, GetNotificationRequest,
    GetNotificationResponse, ListNotificationsRequest, ListNotificationsResponse,
    MarkAllAsReadRequest, MarkAllAsReadResponse, MarkAsReadRequest, MarkAsReadResponse,
    NotificationProto, NotificationType as ProtoNotificationType,
};

/// gRPC NotificationService implementation
#[derive(Clone)]
pub struct NotificationServiceImpl {
    notification_service: Arc<UserNotificationService>,
}

impl NotificationServiceImpl {
    #[must_use]
    pub fn new(notification_service: Arc<UserNotificationService>) -> Self {
        Self {
            notification_service,
        }
    }

    /// Extract user_id from UserContext (injected by inject_user interceptor)
    #[allow(clippy::result_large_err)]
    fn get_user_id(&self, request: &Request<impl std::fmt::Debug>) -> Result<UserId, Status> {
        let user_context = request
            .extensions()
            .get::<super::interceptors::UserContext>()
            .ok_or_else(|| Status::unauthenticated("Authentication required"))?;
        Ok(UserId::from_string(user_context.user_id.clone()))
    }
}

/// Convert a domain Notification to a proto NotificationProto
fn notification_to_proto(n: synctv_core::models::notification::Notification) -> NotificationProto {
    let notification_type = match n.notification_type {
        CoreNotificationType::RoomInvitation => ProtoNotificationType::RoomInvitation,
        CoreNotificationType::SystemAnnouncement => ProtoNotificationType::SystemAnnouncement,
        CoreNotificationType::RoomEvent => ProtoNotificationType::RoomEvent,
        CoreNotificationType::PasswordReset => ProtoNotificationType::PasswordReset,
        CoreNotificationType::EmailVerification => ProtoNotificationType::EmailVerification,
    };

    NotificationProto {
        id: n.id.to_string(),
        user_id: n.user_id.as_str().to_string(),
        notification_type: notification_type as i32,
        title: n.title,
        content: n.content,
        data: serde_json::to_vec(&n.data).unwrap_or_default(),
        is_read: n.is_read,
        created_at: n.created_at.timestamp(),
        updated_at: n.updated_at.timestamp(),
    }
}

/// Convert a proto NotificationType enum value to a domain NotificationType
fn proto_notification_type_to_core(value: i32) -> Option<CoreNotificationType> {
    match ProtoNotificationType::try_from(value) {
        Ok(ProtoNotificationType::RoomInvitation) => Some(CoreNotificationType::RoomInvitation),
        Ok(ProtoNotificationType::SystemAnnouncement) => {
            Some(CoreNotificationType::SystemAnnouncement)
        }
        Ok(ProtoNotificationType::RoomEvent) => Some(CoreNotificationType::RoomEvent),
        Ok(ProtoNotificationType::PasswordReset) => Some(CoreNotificationType::PasswordReset),
        Ok(ProtoNotificationType::EmailVerification) => {
            Some(CoreNotificationType::EmailVerification)
        }
        _ => None, // Unspecified or unknown
    }
}

/// Log and return internal error without leaking details
fn internal_err(context: &str, err: impl std::fmt::Display) -> Status {
    tracing::error!("{context}: {err}");
    Status::internal(context)
}

#[tonic::async_trait]
#[allow(clippy::result_large_err)]
impl NotificationService for NotificationServiceImpl {
    async fn list_notifications(
        &self,
        request: Request<ListNotificationsRequest>,
    ) -> Result<Response<ListNotificationsResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();

        let query = NotificationListQuery {
            page: if req.page > 0 { Some(req.page) } else { Some(1) },
            page_size: Some(req.page_size.clamp(1, 100)),
            is_read: req.is_read,
            notification_type: req
                .notification_type
                .and_then(proto_notification_type_to_core),
        };

        let (notifications, total) = self
            .notification_service
            .list(&user_id, query)
            .await
            .map_err(|e| internal_err("Failed to list notifications", e))?;

        let unread_count = self
            .notification_service
            .get_unread_count(&user_id)
            .await
            .map_err(|e| internal_err("Failed to get unread count", e))?;

        Ok(Response::new(ListNotificationsResponse {
            notifications: notifications.into_iter().map(notification_to_proto).collect(),
            total,
            unread_count,
        }))
    }

    async fn get_notification(
        &self,
        request: Request<GetNotificationRequest>,
    ) -> Result<Response<GetNotificationResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();

        let notification_id = Uuid::parse_str(&req.notification_id)
            .map_err(|_| Status::invalid_argument("Invalid notification_id format"))?;

        let notification = self
            .notification_service
            .get(&user_id, notification_id)
            .await
            .map_err(|e| {
                if e.to_string().contains("not found") {
                    Status::not_found("Notification not found")
                } else {
                    internal_err("Failed to get notification", e)
                }
            })?;

        Ok(Response::new(GetNotificationResponse {
            notification: Some(notification_to_proto(notification)),
        }))
    }

    async fn mark_as_read(
        &self,
        request: Request<MarkAsReadRequest>,
    ) -> Result<Response<MarkAsReadResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();

        let notification_ids: Vec<Uuid> = req
            .notification_ids
            .iter()
            .map(|id| {
                Uuid::parse_str(id)
                    .map_err(|_| Status::invalid_argument(format!("Invalid notification_id: {id}")))
            })
            .collect::<Result<Vec<_>, _>>()?;

        self.notification_service
            .mark_as_read(
                &user_id,
                synctv_core::models::notification::MarkAsReadRequest { notification_ids },
            )
            .await
            .map_err(|e| internal_err("Failed to mark notifications as read", e))?;

        Ok(Response::new(MarkAsReadResponse {}))
    }

    async fn mark_all_as_read(
        &self,
        request: Request<MarkAllAsReadRequest>,
    ) -> Result<Response<MarkAllAsReadResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();

        let before = req
            .before
            .map(|ts| {
                chrono::DateTime::from_timestamp(ts, 0)
                    .ok_or_else(|| Status::invalid_argument("Invalid timestamp"))
            })
            .transpose()?;

        self.notification_service
            .mark_all_as_read(
                &user_id,
                synctv_core::models::notification::MarkAllAsReadRequest { before },
            )
            .await
            .map_err(|e| internal_err("Failed to mark all notifications as read", e))?;

        Ok(Response::new(MarkAllAsReadResponse {}))
    }

    async fn delete_notification(
        &self,
        request: Request<DeleteNotificationRequest>,
    ) -> Result<Response<DeleteNotificationResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();

        let notification_id = Uuid::parse_str(&req.notification_id)
            .map_err(|_| Status::invalid_argument("Invalid notification_id format"))?;

        self.notification_service
            .delete(&user_id, notification_id)
            .await
            .map_err(|e| {
                if e.to_string().contains("not found") {
                    Status::not_found("Notification not found")
                } else {
                    internal_err("Failed to delete notification", e)
                }
            })?;

        Ok(Response::new(DeleteNotificationResponse {}))
    }

    async fn delete_all_read(
        &self,
        request: Request<DeleteAllReadRequest>,
    ) -> Result<Response<DeleteAllReadResponse>, Status> {
        let user_id = self.get_user_id(&request)?;

        self.notification_service
            .delete_all_read(&user_id)
            .await
            .map_err(|e| internal_err("Failed to delete all read notifications", e))?;

        Ok(Response::new(DeleteAllReadResponse {}))
    }
}
