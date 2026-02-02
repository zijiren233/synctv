//! WebRTC HTTP API endpoints
//!
//! Provides REST API for WebRTC signaling and session management.

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::http::AppError;
use synctv_core::{
    models::UserId,
    service::webrtc::{
        SignalingService, MediaType, SessionDescription, IceCandidate, SdpType,
    },
};

/// Create WebRTC router
pub fn create_webrtc_router() -> axum::Router<Arc<super::AppState>> {
    Router::new()
        .route("/servers", get(get_ice_servers))
        .route("/sessions", post(create_session))
        .route("/sessions/:session_id", get(get_session_info).delete(end_session))
        .route("/sessions/:session_id/join", post(join_session))
        .route("/sessions/:session_id/leave", post(leave_session))
        .route("/sessions/:session_id/offer", post(handle_offer))
        .route("/sessions/:session_id/answer", post(handle_answer))
        .route("/sessions/:session_id/ice", post(handle_ice_candidate))
}

/// Get ICE server configuration
///
/// Returns STUN/TURN server configuration for WebRTC clients.
#[utoipa::path(
    get,
    path = "/api/webrtc/servers",
    tag = "webrtc",
    responses(
        (status = 200, description = "ICE server configuration", body = IceServersResponse),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
async fn get_ice_servers(State(state): State<Arc<super::AppState>>) -> Result<Json<IceServersResponse>, AppError> {
    let signaling_service = state
        .webrtc_service
        .as_ref()
        .ok_or_else(|| AppError::internal("WebRTC service not available"))?;

    let ice_servers = signaling_service.get_ice_servers();

    Ok(Json(IceServersResponse {
        stun_servers: ice_servers.stun_servers,
        turn_config: ice_servers.turn_config,
    }))
}

/// Create a new WebRTC session
///
/// Creates a new WebRTC session (call) for a room.
#[utoipa::path(
    post,
    path = "/api/webrtc/sessions",
    tag = "webrtc",
    request_body = CreateSessionRequest,
    responses(
        (status = 200, description = "Session created successfully", body = CreateSessionResponse),
        (status = 400, description = "Invalid request"),
        (status = 409, description = "Session already exists for this room"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
async fn create_session(
    State(state): State<Arc<super::AppState>>,
    Json(req): Json<CreateSessionRequest>,
    auth_user: super::middleware::AuthUser,
) -> Result<Json<CreateSessionResponse>, AppError> {
    let signaling_service = state
        .webrtc_service
        .as_ref()
        .ok_or_else(|| AppError::internal("WebRTC service not available"))?;

    let response = signaling_service
        .create_session(req.room_id, req.media_type, auth_user.user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to create session: {}", e)))?;

    Ok(Json(CreateSessionResponse {
        session_id: response.session_id,
        ice_servers: IceServersResponse {
            stun_servers: response.ice_servers.stun_servers,
            turn_config: response.ice_servers.turn_config,
        },
    }))
}

/// Get session information
///
/// Returns information about a WebRTC session.
#[utoipa::path(
    get,
    path = "/api/webrtc/sessions/{session_id}",
    tag = "webrtc",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Session information", body = SessionInfoResponse),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
async fn get_session_info(
    State(state): State<Arc<super::AppState>>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionInfoResponse>, AppError> {
    let signaling_service = state
        .webrtc_service
        .as_ref()
        .ok_or_else(|| AppError::internal("WebRTC service not available"))?;

    let session_info = signaling_service
        .get_session_info(&session_id)
        .await
        .map_err(|e| AppError::not_found(format!("Session not found: {}", e)))?;

    Ok(Json(SessionInfoResponse::from(session_info)))
}

/// Join a WebRTC session
///
/// Join an existing WebRTC session as a participant.
#[utoipa::path(
    post,
    path = "/api/webrtc/sessions/{session_id}/join",
    tag = "webrtc",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = JoinSessionRequest,
    responses(
        (status = 200, description = "Joined session successfully", body = JoinSessionResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 409, description = "Session is full or user already in session"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
async fn join_session(
    State(state): State<Arc<super::AppState>>,
    Path(session_id): Path<String>,
    Json(req): Json<JoinSessionRequest>,
    auth_user: super::middleware::AuthUser,
) -> Result<Json<JoinSessionResponse>, AppError> {
    let signaling_service = state
        .webrtc_service
        .as_ref()
        .ok_or_else(|| AppError::internal("WebRTC service not available"))?;

    let response = signaling_service
        .join_session(&session_id, auth_user.user_id, req.username)
        .await
        .map_err(|e| AppError::bad_request(format!("Failed to join session: {}", e)))?;

    Ok(Json(JoinSessionResponse::from(response)))
}

/// Leave a WebRTC session
///
/// Leave a WebRTC session.
#[utoipa::path(
    post,
    path = "/api/webrtc/sessions/{session_id}/leave",
    tag = "webrtc",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = LeaveSessionRequest,
    responses(
        (status = 200, description = "Left session successfully"),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
async fn leave_session(
    State(state): State<Arc<super::AppState>>,
    Path(session_id): Path<String>,
    Json(req): Json<LeaveSessionRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let signaling_service = state
        .webrtc_service
        .as_ref()
        .ok_or_else(|| AppError::internal("WebRTC service not available"))?;

    signaling_service
        .leave_session(&session_id, &req.peer_id)
        .await
        .map_err(|e| AppError::bad_request(format!("Failed to leave session: {}", e)))?;

    Ok(Json(serde_json::json!({
        "success": true
    })))
}

/// Handle WebRTC offer
///
/// Process a WebRTC offer from a peer.
#[utoipa::path(
    post,
    path = "/api/webrtc/sessions/{session_id}/offer",
    tag = "webrtc",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = OfferRequest,
    responses(
        (status = 200, description = "Offer processed successfully"),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
async fn handle_offer(
    State(state): State<Arc<super::AppState>>,
    Path(session_id): Path<String>,
    Json(req): Json<OfferRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let signaling_service = state
        .webrtc_service
        .as_ref()
        .ok_or_else(|| AppError::internal("WebRTC service not available"))?;

    signaling_service
        .handle_offer(&session_id, &req.peer_id, req.sdp)
        .await
        .map_err(|e| AppError::bad_request(format!("Failed to handle offer: {}", e)))?;

    Ok(Json(serde_json::json!({
        "success": true
    })))
}

/// Handle WebRTC answer
///
/// Process a WebRTC answer from a peer.
#[utoipa::path(
    post,
    path = "/api/webrtc/sessions/{session_id}/answer",
    tag = "webrtc",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = AnswerRequest,
    responses(
        (status = 200, description = "Answer processed successfully"),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
async fn handle_answer(
    State(state): State<Arc<super::AppState>>,
    Path(session_id): Path<String>,
    Json(req): Json<AnswerRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let signaling_service = state
        .webrtc_service
        .as_ref()
        .ok_or_else(|| AppError::internal("WebRTC service not available"))?;

    signaling_service
        .handle_answer(&session_id, &req.peer_id, req.sdp)
        .await
        .map_err(|e| AppError::bad_request(format!("Failed to handle answer: {}", e)))?;

    Ok(Json(serde_json::json!({
        "success": true
    })))
}

/// Handle ICE candidate
///
/// Process an ICE candidate from a peer.
#[utoipa::path(
    post,
    path = "/api/webrtc/sessions/{session_id}/ice",
    tag = "webrtc",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = IceCandidateRequest,
    responses(
        (status = 200, description = "ICE candidate processed successfully"),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
async fn handle_ice_candidate(
    State(state): State<Arc<super::AppState>>,
    Path(session_id): Path<String>,
    Json(req): Json<IceCandidateRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let signaling_service = state
        .webrtc_service
        .as_ref()
        .ok_or_else(|| AppError::internal("WebRTC service not available"))?;

    signaling_service
        .handle_ice_candidate(&session_id, &req.peer_id, req.candidate)
        .await
        .map_err(|e| AppError::bad_request(format!("Failed to handle ICE candidate: {}", e)))?;

    Ok(Json(serde_json::json!({
        "success": true
    })))
}

/// End a WebRTC session
///
/// End a WebRTC session and remove all participants.
#[utoipa::path(
    delete,
    path = "/api/webrtc/sessions/{session_id}",
    tag = "webrtc",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Session ended successfully"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
async fn end_session(
    State(state): State<Arc<super::AppState>>,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let signaling_service = state
        .webrtc_service
        .as_ref()
        .ok_or_else(|| AppError::internal("WebRTC service not available"))?;

    signaling_service
        .end_session(&session_id)
        .await
        .map_err(|e| AppError::bad_request(format!("Failed to end session: {}", e)))?;

    Ok(Json(serde_json::json!({
        "success": true
    })))
}

// Request/Response types

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub room_id: String,
    pub media_type: MediaType,
}

#[derive(Debug, Serialize)]
pub struct IceServersResponse {
    pub stun_servers: Vec<String>,
    pub turn_config: Option<synctv_core::service::webrtc::TurnConfig>,
}

#[derive(Debug, Serialize)]
pub struct CreateSessionResponse {
    pub session_id: String,
    pub ice_servers: IceServersResponse,
}

#[derive(Debug, Serialize)]
pub struct SessionInfoResponse {
    pub session_id: String,
    pub room_id: String,
    pub state: synctv_core::service::webrtc::session::SessionState,
    pub media_type: MediaType,
    pub peer_count: usize,
    pub peers: Vec<synctv_core::service::webrtc::Peer>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<synctv_core::service::webrtc::signaling::SessionInfo> for SessionInfoResponse {
    fn from(info: synctv_core::service::webrtc::signaling::SessionInfo) -> Self {
        Self {
            session_id: info.session_id,
            room_id: info.room_id,
            state: info.state,
            media_type: info.media_type,
            peer_count: info.peer_count,
            peers: info.peers,
            created_at: info.created_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct JoinSessionRequest {
    pub username: String,
}

#[derive(Debug, Serialize)]
pub struct JoinSessionResponse {
    pub peer_id: String,
    pub peers: Vec<synctv_core::service::webrtc::Peer>,
    pub session_state: synctv_core::service::webrtc::session::SessionState,
}

impl From<synctv_core::service::webrtc::signaling::JoinSessionResponse> for JoinSessionResponse {
    fn from(resp: synctv_core::service::webrtc::signaling::JoinSessionResponse) -> Self {
        Self {
            peer_id: resp.peer_id,
            peers: resp.peers,
            session_state: resp.session_state,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct LeaveSessionRequest {
    pub peer_id: String,
}

#[derive(Debug, Deserialize)]
pub struct OfferRequest {
    pub peer_id: String,
    pub sdp: SessionDescription,
}

#[derive(Debug, Deserialize)]
pub struct AnswerRequest {
    pub peer_id: String,
    pub sdp: SessionDescription,
}

#[derive(Debug, Deserialize)]
pub struct IceCandidateRequest {
    pub peer_id: String,
    pub candidate: IceCandidate,
}
