//! User notification HTTP endpoints
//!
//! REST API for managing user notifications

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::http::error::AppResult;
use crate::http::middleware::AuthUser;
use crate::http::AppState;
use synctv_core::models::notification::{
    Notification, NotificationListQuery,
};

/// List notifications response
#[derive(Debug, Serialize)]
pub struct ListNotificationsResponse {
    pub notifications: Vec<Notification>,
    pub total: i64,
    pub unread_count: i64,
}

/// Mark as read request
#[derive(Debug, Deserialize)]
pub struct MarkAsReadRequest {
    pub notification_ids: Vec<Uuid>,
}

/// Mark all as read request
#[derive(Debug, Deserialize)]
pub struct MarkAllAsReadRequest {
    pub before: Option<chrono::DateTime<chrono::Utc>>,
}

/// Query parameters for listing notifications
#[derive(Debug, Deserialize)]
pub struct ListNotificationsQuery {
    pub page: Option<i32>,
    pub page_size: Option<i32>,
    pub is_read: Option<bool>,
    pub notification_type: Option<String>,
}

/// GET /api/notifications - List user's notifications
pub async fn list_notifications(
    auth: AuthUser,
    Query(query): Query<ListNotificationsQuery>,
    State(state): State<AppState>,
) -> AppResult<Json<ListNotificationsResponse>> {
    // Parse notification type from string if provided
    let notification_type = query
        .notification_type
        .and_then(|t| t.parse().ok());

    let query = NotificationListQuery {
        page: query.page,
        page_size: query.page_size,
        is_read: query.is_read,
        notification_type,
    };

    let notification_service = state.notification_service.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Notification service not configured"))?;

    let (notifications, total) = notification_service
        .list(&auth.user_id, query)
        .await?;

    let unread_count = notification_service
        .get_unread_count(&auth.user_id)
        .await?;

    Ok(Json(ListNotificationsResponse {
        notifications,
        total,
        unread_count,
    }))
}

/// GET /api/notifications/:id - Get a specific notification
pub async fn get_notification(
    auth: AuthUser,
    Path(notification_id): Path<Uuid>,
    State(state): State<AppState>,
) -> AppResult<Json<Notification>> {
    let notification_service = state.notification_service.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Notification service not configured"))?;

    let notification = notification_service
        .get(&auth.user_id, notification_id)
        .await?;

    Ok(Json(notification))
}

/// POST /api/notifications/read - Mark notifications as read
pub async fn mark_as_read(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<MarkAsReadRequest>,
) -> AppResult<StatusCode> {
    let notification_service = state.notification_service.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Notification service not configured"))?;

    notification_service
        .mark_as_read(&auth.user_id, synctv_core::models::notification::MarkAsReadRequest {
            notification_ids: req.notification_ids,
        })
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/notifications/read-all - Mark all notifications as read
pub async fn mark_all_as_read(
    auth: AuthUser,
    State(state): State<AppState>,
    req: Option<Json<MarkAllAsReadRequest>>,
) -> AppResult<StatusCode> {
    let notification_service = state.notification_service.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Notification service not configured"))?;

    let before = req.and_then(|r| r.before);

    notification_service
        .mark_all_as_read(&auth.user_id, synctv_core::models::notification::MarkAllAsReadRequest {
            before,
        })
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/notifications/:id - Delete a notification
pub async fn delete_notification(
    auth: AuthUser,
    Path(notification_id): Path<Uuid>,
    State(state): State<AppState>,
) -> AppResult<StatusCode> {
    let notification_service = state.notification_service.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Notification service not configured"))?;

    notification_service
        .delete(&auth.user_id, notification_id)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/notifications/read - Delete all read notifications
pub async fn delete_all_read(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<StatusCode> {
    let notification_service = state.notification_service.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Notification service not configured"))?;

    notification_service
        .delete_all_read(&auth.user_id)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Create the notification router
pub fn create_notification_router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/api/notifications", axum::routing::get(list_notifications))
        .route("/api/notifications/:id", axum::routing::get(get_notification))
        .route(
            "/api/notifications/read",
            axum::routing::post(mark_as_read),
        )
        .route(
            "/api/notifications/read-all",
            axum::routing::post(mark_all_as_read),
        )
        .route(
            "/api/notifications/:id",
            axum::routing::delete(delete_notification),
        )
        .route(
            "/api/notifications/read",
            axum::routing::delete(delete_all_read),
        )
}
