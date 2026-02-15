//! User notification HTTP endpoints
//!
//! REST API for managing user notifications.
//! Delegates to NotificationApiImpl for shared business logic.

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

/// List notifications response
#[derive(Debug, Serialize)]
pub struct ListNotificationsResponse {
    pub notifications: Vec<synctv_core::models::notification::Notification>,
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

fn get_notification_api(state: &AppState) -> Result<&crate::impls::NotificationApiImpl, crate::http::AppError> {
    state.notification_api.as_ref()
        .map(|arc| arc.as_ref())
        .ok_or_else(|| crate::http::AppError::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "Notification service not available",
        ))
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
        .and_then(|t| t.parse().ok());

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
        notifications: result.notifications,
        total: result.total,
        unread_count: result.unread_count,
    }))
}

/// GET /api/notifications/:id - Get a specific notification
pub async fn get_notification(
    auth: AuthUser,
    Path(notification_id): Path<Uuid>,
    State(state): State<AppState>,
) -> AppResult<Json<synctv_core::models::notification::Notification>> {
    let api = get_notification_api(&state)?;

    let notification = api
        .get_notification(&auth.user_id, notification_id)
        .await
        .map_err(crate::http::AppError::internal)?;

    Ok(Json(notification))
}

/// POST /api/notifications/read - Mark notifications as read
pub async fn mark_as_read(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<MarkAsReadRequest>,
) -> AppResult<StatusCode> {
    let api = get_notification_api(&state)?;

    api.mark_as_read(&auth.user_id, req.notification_ids)
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

    let before = req.and_then(|r| r.before);

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
