//! User notification HTTP endpoints
//!
//! REST API for managing user notifications.
//! Delegates to NotificationApiImpl for shared business logic.
//!
//! Uses proto-generated types for request/response to ensure type consistency
//! with gRPC handlers.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::http::error::AppResult;
use crate::http::middleware::AuthUser;
use crate::http::AppState;
use crate::proto::client::{
    ListNotificationsResponse, NotificationProto,
    NotificationType as ProtoNotificationType,
    MarkAsReadRequest, MarkAllAsReadRequest,
    GetNotificationResponse,
};

/// Query parameters for listing notifications (HTTP-specific, not in proto)
#[derive(Debug, Deserialize)]
pub struct ListNotificationsQuery {
    pub page: Option<i32>,
    pub page_size: Option<i32>,
    pub is_read: Option<bool>,
    pub notification_type: Option<String>,
}

fn get_notification_api(state: &AppState) -> Result<&crate::impls::NotificationApiImpl, crate::http::AppError> {
    state.notification_api.as_ref()
        .map(|arc| arc.as_ref())
        .ok_or_else(|| crate::http::AppError::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "Notification service not available",
        ))
}

/// Convert a domain Notification to a proto NotificationProto
fn notification_to_proto(n: synctv_core::models::notification::Notification) -> NotificationProto {
    use synctv_core::models::notification::NotificationType as CoreNotificationType;

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
fn proto_notification_type_to_core(value: i32) -> Option<synctv_core::models::notification::NotificationType> {
    use synctv_core::models::notification::NotificationType as CoreNotificationType;

    match ProtoNotificationType::try_from(value) {
        Ok(ProtoNotificationType::RoomInvitation) => Some(CoreNotificationType::RoomInvitation),
        Ok(ProtoNotificationType::SystemAnnouncement) => Some(CoreNotificationType::SystemAnnouncement),
        Ok(ProtoNotificationType::RoomEvent) => Some(CoreNotificationType::RoomEvent),
        Ok(ProtoNotificationType::PasswordReset) => Some(CoreNotificationType::PasswordReset),
        Ok(ProtoNotificationType::EmailVerification) => Some(CoreNotificationType::EmailVerification),
        _ => None,
    }
}

/// GET /api/notifications - List user's notifications
pub async fn list_notifications(
    auth: AuthUser,
    Query(query): Query<ListNotificationsQuery>,
    State(state): State<AppState>,
) -> AppResult<Json<ListNotificationsResponse>> {
    let api = get_notification_api(&state)?;

    let notification_type = query
        .notification_type
        .as_deref()
        .and_then(|t| t.parse::<i32>().ok())
        .and_then(proto_notification_type_to_core);

    let result = api
        .list_notifications(
            &auth.user_id,
            query.page,
            query.page_size,
            query.is_read,
            notification_type,
        )
        .await
        .map_err(crate::http::AppError::internal)?;

    Ok(Json(ListNotificationsResponse {
        notifications: result.notifications.into_iter().map(notification_to_proto).collect(),
        total: result.total,
        unread_count: result.unread_count,
    }))
}

/// GET /api/notifications/:id - Get a specific notification
pub async fn get_notification(
    auth: AuthUser,
    Path(notification_id): Path<Uuid>,
    State(state): State<AppState>,
) -> AppResult<Json<GetNotificationResponse>> {
    let api = get_notification_api(&state)?;

    let notification = api
        .get_notification(&auth.user_id, notification_id)
        .await
        .map_err(crate::http::AppError::internal)?;

    Ok(Json(GetNotificationResponse {
        notification: Some(notification_to_proto(notification)),
    }))
}

/// POST /api/notifications/read - Mark notifications as read
pub async fn mark_as_read(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<MarkAsReadRequest>,
) -> AppResult<StatusCode> {
    let api = get_notification_api(&state)?;

    let notification_ids: Vec<Uuid> = req
        .notification_ids
        .iter()
        .map(|id| {
            Uuid::parse_str(id)
                .map_err(|_| crate::http::AppError::bad_request(format!("Invalid notification_id: {id}")))
        })
        .collect::<Result<Vec<_>, _>>()?;

    api.mark_as_read(&auth.user_id, notification_ids)
        .await
        .map_err(crate::http::AppError::internal)?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/notifications/read-all - Mark all notifications as read
pub async fn mark_all_as_read(
    auth: AuthUser,
    State(state): State<AppState>,
    req: Option<Json<MarkAllAsReadRequest>>,
) -> AppResult<StatusCode> {
    let api = get_notification_api(&state)?;

    let before = req
        .and_then(|r| r.before)
        .map(|ts| {
            chrono::DateTime::from_timestamp(ts, 0)
                .ok_or_else(|| crate::http::AppError::bad_request("Invalid timestamp"))
        })
        .transpose()?;

    api.mark_all_as_read(&auth.user_id, before)
        .await
        .map_err(crate::http::AppError::internal)?;

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/notifications/:id - Delete a notification
pub async fn delete_notification(
    auth: AuthUser,
    Path(notification_id): Path<Uuid>,
    State(state): State<AppState>,
) -> AppResult<StatusCode> {
    let api = get_notification_api(&state)?;

    api.delete_notification(&auth.user_id, notification_id)
        .await
        .map_err(crate::http::AppError::internal)?;

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/notifications/read - Delete all read notifications
pub async fn delete_all_read(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<StatusCode> {
    let api = get_notification_api(&state)?;

    api.delete_all_read(&auth.user_id)
        .await
        .map_err(crate::http::AppError::internal)?;

    Ok(StatusCode::NO_CONTENT)
}

/// Create the notification read router (GET endpoints -- under read rate limit)
pub fn create_notification_read_router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/api/notifications", axum::routing::get(list_notifications))
        .route(
            "/api/notifications/:id",
            axum::routing::get(get_notification),
        )
}

/// Create the notification write router (POST/DELETE endpoints -- under write rate limit)
pub fn create_notification_write_router() -> axum::Router<AppState> {
    axum::Router::new()
        .route(
            "/api/notifications/:id",
            axum::routing::delete(delete_notification),
        )
        .route(
            "/api/notifications/actions/mark-read",
            axum::routing::post(mark_as_read).delete(delete_all_read),
        )
        .route(
            "/api/notifications/read-all",
            axum::routing::post(mark_all_as_read),
        )
}
