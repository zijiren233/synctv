//! RTMP authentication implementation for `SyncTV`
//!
//! This module provides the RTMP authentication callback that integrates
//! with `SyncTV`'s user and room management.
//!
//! On successful publish auth:
//! 1. Atomically registers the publisher in Redis (single-publisher-per-media enforcement)
//! 2. Registers the user→stream mapping in the local `StreamTracker`
//! 3. Spawns a background TTL renewal task for the Redis registration
//!
//! On unpublish:
//! 1. Aborts the TTL renewal task
//! 2. Unregisters the publisher from Redis
//! 3. Removes the user→stream mapping from the local `StreamTracker`

use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use synctv_xiu::rtmp::auth::AuthCallback;
use synctv_livestream::api::UserStreamTracker;
use synctv_livestream::relay::StreamRegistryTrait;
use tokio::task::AbortHandle;

use synctv_core::{
    models::{RoomStatus, UserStatus, MediaId, UserId},
    service::{PublishKeyService, RoomService, UserService},
};

/// Stream lifecycle event emitted on publish/unpublish
#[derive(Debug, Clone)]
pub enum StreamLifecycleEvent {
    /// A publisher successfully started streaming
    Started {
        room_id: String,
        media_id: String,
        user_id: String,
    },
    /// A publisher stopped streaming
    Stopped {
        room_id: String,
        media_id: String,
        user_id: String,
    },
}

/// RTMP authentication implementation for `SyncTV`
///
/// Validates RTMP publish/play requests against:
/// - Room existence and status (not banned/pending)
/// - JWT publish keys for publishers (validates `room_id` match)
/// - User status (not banned/deleted)
/// - Authorization (global admin, room admin/creator, or media creator)
/// - Single-publisher-per-media (atomic Redis registration)
/// - RTMP pull (play) is unconditionally rejected — viewers must use HTTP-FLV or HLS
///
/// On successful publish auth, registers the publisher in Redis and
/// spawns a TTL renewal task. On unpublish, cleans up Redis and tracker state.
pub struct SyncTvRtmpAuth {
    room_service: Arc<RoomService>,
    user_service: Arc<UserService>,
    publish_key_service: Arc<PublishKeyService>,
    user_stream_tracker: UserStreamTracker,
    /// Publisher registry (Redis) for single-publisher-per-media enforcement
    registry: Arc<dyn StreamRegistryTrait>,
    /// This node's unique identifier for publisher registration
    node_id: String,
    /// Broadcast channel for stream lifecycle events (StreamStarted/StreamStopped)
    stream_event_tx: Option<tokio::sync::broadcast::Sender<StreamLifecycleEvent>>,
    /// Active TTL renewal tasks: "`room_id:media_id`" → `AbortHandle`
    ttl_handles: DashMap<String, AbortHandle>,
}

impl SyncTvRtmpAuth {
    pub fn new(
        room_service: Arc<RoomService>,
        user_service: Arc<UserService>,
        publish_key_service: Arc<PublishKeyService>,
        user_stream_tracker: UserStreamTracker,
        registry: Arc<dyn StreamRegistryTrait>,
        node_id: String,
        stream_event_tx: Option<tokio::sync::broadcast::Sender<StreamLifecycleEvent>>,
    ) -> Self {
        Self {
            room_service,
            user_service,
            publish_key_service,
            user_stream_tracker,
            registry,
            node_id,
            stream_event_tx,
            ttl_handles: DashMap::new(),
        }
    }
}

#[async_trait]
impl AuthCallback for SyncTvRtmpAuth {
    async fn on_publish(
        &self,
        app_name: &str,
        stream_name: &str,
        query: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // app_name = room_id
        // stream_name = JWT token (or media_id if token is in query)
        // query = optional query string (may contain token=JWT)

        // Validate room
        let room = self
            .room_service
            .get_room(&synctv_core::models::RoomId::from_string(app_name.to_string()))
            .await
            .map_err(|e| format!("Failed to load room: {e}"))?;

        if room.status == RoomStatus::Banned {
            return Err(format!("Room {app_name} is banned").into());
        }

        if room.status == RoomStatus::Pending {
            return Err(format!("Room {app_name} is pending, need admin approval").into());
        }

        // Extract token: prefer query string parameter, fall back to stream_name as token
        let token = if let Some(q) = query {
            extract_token_from_query(q).unwrap_or(stream_name)
        } else {
            stream_name
        };

        // Validate JWT stream_key
        let claims = self
            .publish_key_service
            .validate_publish_key(token)
            .await
            .map_err(|e| format!("Invalid stream key: {e}"))?;

        // Verify room_id matches
        if claims.room_id != app_name {
            return Err(format!(
                "Room ID mismatch: token for room {}, but connecting to room {}",
                claims.room_id, app_name
            )
            .into());
        }

        // Re-verify user status at connection time (permissions may have changed since JWT generation)
        let user_id = UserId::from_string(claims.user_id.clone());
        let user = self
            .user_service
            .get_user(&user_id)
            .await
            .map_err(|e| format!("Failed to load user: {e}"))?;

        if user.status == UserStatus::Banned {
            return Err(format!("User {} is banned", claims.user_id).into());
        }
        if user.deleted_at.is_some() {
            return Err(format!("User {} is deleted", claims.user_id).into());
        }

        // Authorization check: user must be global admin, room admin/creator, or media creator
        let is_global_admin = user.role.is_admin_or_above();

        let is_room_admin_or_creator = if is_global_admin {
            false
        } else {
            let room_id = synctv_core::models::RoomId::from_string(app_name.to_string());
            match self.room_service.member_service().get_member(&room_id, &user_id).await {
                Ok(Some(member)) => matches!(
                    member.role,
                    synctv_core::models::RoomRole::Creator | synctv_core::models::RoomRole::Admin
                ),
                _ => false,
            }
        };

        // Verify media belongs to this room
        let media_id = MediaId::from_string(claims.media_id.clone());
        let room_id_obj = synctv_core::models::RoomId::from_string(app_name.to_string());
        let media = self.room_service.media_service().get_media(&media_id).await
            .map_err(|e| format!("Failed to load media: {e}"))?
            .ok_or_else(|| format!("Media {} not found", claims.media_id))?;
        if media.room_id != room_id_obj {
            return Err(format!(
                "Media {} does not belong to room {}",
                claims.media_id, app_name
            ).into());
        }

        let is_media_creator = if !is_global_admin && !is_room_admin_or_creator {
            media.creator_id == user_id
        } else {
            false
        };

        if !is_global_admin && !is_room_admin_or_creator && !is_media_creator {
            return Err(format!(
                "Insufficient permissions to publish: user {} is not admin, room admin/creator, or media creator",
                claims.user_id
            ).into());
        }

        // Enforce single-publisher-per-media: atomically register in Redis
        let registered = self.registry
            .try_register_publisher(
                &claims.room_id,
                &claims.media_id,
                &self.node_id,
                &claims.user_id,
            )
            .await
            .map_err(|e| format!("Failed to register publisher in Redis: {e}"))?;

        if !registered {
            return Err(format!(
                "Another publisher is already active for media {} in room {}",
                claims.media_id, claims.room_id
            ).into());
        }

        tracing::info!(
            "Publisher authenticated and registered: user={}, room={}, media={}, node={}, auth={}",
            claims.user_id,
            app_name,
            claims.media_id,
            self.node_id,
            if is_global_admin { "global_admin" }
            else if is_room_admin_or_creator { "room_admin" }
            else { "media_creator" },
        );

        // Track user→stream mapping for kick-on-ban (with RTMP identifier mapping)
        self.user_stream_tracker.insert(
            claims.user_id.clone(),
            app_name.to_string(),
            claims.media_id.clone(),
            app_name,
            stream_name,
        );

        // Emit stream lifecycle event
        if let Some(ref tx) = self.stream_event_tx {
            let _ = tx.send(StreamLifecycleEvent::Started {
                room_id: claims.room_id.clone(),
                media_id: claims.media_id.clone(),
                user_id: claims.user_id.clone(),
            });
        }

        // Spawn TTL renewal task (refreshes Redis registration every 60 seconds)
        let ttl_key = format!("{}:{}", claims.room_id, claims.media_id);
        let registry = self.registry.clone();
        let room_id = claims.room_id.clone();
        let media_id = claims.media_id.clone();
        let ttl_user_id = claims.user_id.clone();
        let ttl_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_mins(1));
            // Skip the first immediate tick
            interval.tick().await;
            loop {
                interval.tick().await;
                if let Err(e) = registry.refresh_publisher_ttl(&room_id, &media_id, &ttl_user_id).await {
                    tracing::error!(
                        room_id = %room_id,
                        media_id = %media_id,
                        "Failed to refresh publisher TTL: {}",
                        e
                    );
                    break;
                }
                tracing::trace!(
                    room_id = %room_id,
                    media_id = %media_id,
                    "Refreshed publisher TTL"
                );
            }
        });
        self.ttl_handles.insert(ttl_key, ttl_task.abort_handle());

        Ok(())
    }

    /// RTMP pull (play) is unconditionally rejected.
    ///
    /// All viewer access must go through HTTP-FLV (`/api/room/movie/live/flv/`) or
    /// HLS (`/api/room/movie/live/hls/`) endpoints, which enforce JWT + room membership auth.
    async fn on_play(
        &self,
        app_name: &str,
        stream_name: &str,
        _query: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        tracing::warn!(
            room_id = %app_name,
            media_id = %stream_name,
            "RTMP play rejected: direct RTMP pull is disabled, use HTTP-FLV or HLS"
        );
        Err("RTMP pull is disabled. Use HTTP-FLV or HLS endpoints for playback.".into())
    }

    async fn on_unplay(
        &self,
        app_name: &str,
        stream_name: &str,
        _query: Option<&str>,
    ) {
        tracing::info!(
            room_id = %app_name,
            media_id = %stream_name,
            "RTMP player disconnected"
        );
    }

    async fn on_unpublish(
        &self,
        app_name: &str,
        stream_name: &str,
        _query: Option<&str>,
    ) {
        // Remove user→stream mapping from tracker (resolves RTMP identifiers to logical stream)
        if let Some((user_id, room_id, media_id)) =
            self.user_stream_tracker.remove_by_app_stream(app_name, stream_name)
        {
            tracing::info!(
                user_id = %user_id,
                room_id = %room_id,
                media_id = %media_id,
                "Publisher unpublished, cleaning up"
            );

            // Abort TTL renewal task
            let ttl_key = format!("{room_id}:{media_id}");
            if let Some((_, handle)) = self.ttl_handles.remove(&ttl_key) {
                handle.abort();
            }

            // Unregister from Redis
            if let Err(e) = self.registry.unregister_publisher(&room_id, &media_id).await {
                tracing::error!(
                    room_id = %room_id,
                    media_id = %media_id,
                    "Failed to unregister publisher from Redis: {}",
                    e
                );
            }

            // Emit stream lifecycle event
            if let Some(ref tx) = self.stream_event_tx {
                let _ = tx.send(StreamLifecycleEvent::Stopped {
                    room_id: room_id.clone(),
                    media_id: media_id.clone(),
                    user_id: user_id.clone(),
                });
            }
        } else {
            tracing::warn!(
                app_name = %app_name,
                stream_name = %stream_name,
                "on_unpublish: no matching stream found in tracker"
            );
        }
    }
}

fn extract_token_from_query(query: &str) -> Option<&str> {
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("token=") {
            return Some(value);
        }
    }
    None
}
