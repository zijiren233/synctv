//! Client API Implementation
//!
//! Unified implementation for all client API operations.
//! Used by both HTTP and gRPC handlers.

use std::sync::Arc;
use std::str::FromStr;
use synctv_core::models::{UserId, RoomId, ProviderType};
use synctv_core::service::{UserService, RoomService};

/// Client API implementation
#[derive(Clone)]
pub struct ClientApiImpl {
    pub user_service: Arc<UserService>,
    pub room_service: Arc<RoomService>,
}

impl ClientApiImpl {
    pub fn new(
        user_service: Arc<UserService>,
        room_service: Arc<RoomService>,
    ) -> Self {
        Self {
            user_service,
            room_service,
        }
    }

    // === Auth Operations ===

    pub async fn register(
        &self,
        _req: crate::proto::client::RegisterRequest,
    ) -> Result<crate::proto::client::RegisterResponse, String> {
        // TODO: Implement using user_service.register()
        Err("Not implemented".to_string())
    }

    pub async fn login(
        &self,
        _req: crate::proto::client::LoginRequest,
    ) -> Result<crate::proto::client::LoginResponse, String> {
        // TODO: Implement using user_service.login()
        Err("Not implemented".to_string())
    }

    pub async fn refresh_token(
        &self,
        _req: crate::proto::client::RefreshTokenRequest,
    ) -> Result<crate::proto::client::RefreshTokenResponse, String> {
        Err("Not implemented".to_string())
    }

    pub async fn logout(
        &self,
        _req: crate::proto::client::LogoutRequest,
    ) -> Result<crate::proto::client::LogoutResponse, String> {
        Err("Not implemented".to_string())
    }

    pub async fn get_current_user(
        &self,
        user_id: &str,
    ) -> Result<crate::proto::client::GetCurrentUserResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let user = self.user_service.get_user(&uid).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::GetCurrentUserResponse {
            user: Some(user_to_proto(&user)),
        })
    }

    pub async fn update_username(
        &self,
        user_id: &str,
        req: crate::proto::client::UpdateUsernameRequest,
    ) -> Result<crate::proto::client::UpdateUsernameResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let user = self.user_service.get_user(&uid).await
            .map_err(|e| e.to_string())?;

        let updated_user = synctv_core::models::User {
            username: req.new_username,
            ..user
        };

        let result_user = self.user_service.update_user(&updated_user).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::UpdateUsernameResponse {
            user: Some(user_to_proto(&result_user)),
        })
    }

    pub async fn update_password(
        &self,
        user_id: &str,
        req: crate::proto::client::UpdatePasswordRequest,
    ) -> Result<crate::proto::client::UpdatePasswordResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        self.user_service.update_password(&uid, &req.new_password).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::UpdatePasswordResponse {
            success: true,
        })
    }

    // === Room Operations ===

    pub async fn list_rooms(
        &self,
        _req: crate::proto::client::ListRoomsRequest,
    ) -> Result<crate::proto::client::ListRoomsResponse, String> {
        let query = synctv_core::models::RoomListQuery::default();
        let (rooms, total) = self.room_service.list_rooms(&query).await
            .map_err(|e| e.to_string())?;

        let room_list: Vec<_> = rooms.into_iter().map(|r| room_to_proto_basic(&r)).collect();

        Ok(crate::proto::client::ListRoomsResponse {
            rooms: room_list,
            total: total as i32,
        })
    }

    pub async fn get_joined_rooms(
        &self,
        user_id: &str,
    ) -> Result<crate::proto::client::GetJoinedRoomsResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let (rooms, total) = self.room_service.list_joined_rooms_with_details(&uid, 1, 100).await
            .map_err(|e| e.to_string())?;

        let room_list: Vec<_> = rooms.into_iter().map(|r| room_to_proto_basic(&r)).collect();

        Ok(crate::proto::client::GetJoinedRoomsResponse {
            rooms: room_list,
            total: total as i32,
        })
    }

    pub async fn create_room(
        &self,
        user_id: &str,
        req: crate::proto::client::CreateRoomRequest,
    ) -> Result<crate::proto::client::CreateRoomResponse, String> {
        let uid = UserId::from_string(user_id.to_string());

        let settings = if !req.settings.is_empty() {
            Some(serde_json::from_slice(&req.settings).map_err(|e| e.to_string())?)
        } else {
            None
        };

        let password = if req.password.is_empty() { None } else { Some(req.password) };

        let (room, _member) = self.room_service.create_room(req.name, uid, password, settings).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::CreateRoomResponse {
            room: Some(room_to_proto_basic(&room)),
        })
    }

    pub async fn get_room(
        &self,
        room_id: &str,
    ) -> Result<crate::proto::client::GetRoomResponse, String> {
        let rid = RoomId::from_string(room_id.to_string());
        let room = self.room_service.get_room(&rid).await
            .map_err(|e| e.to_string())?;

        let playback_state = self.room_service.get_playback_state(&rid).await.ok()
            .map(playback_state_to_proto);

        Ok(crate::proto::client::GetRoomResponse {
            room: Some(room_to_proto_basic(&room)),
            playback_state,
        })
    }

    pub async fn join_room(
        &self,
        user_id: &str,
        req: crate::proto::client::JoinRoomRequest,
    ) -> Result<crate::proto::client::JoinRoomResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(req.room_id.clone());

        let password = if req.password.is_empty() { None } else { Some(req.password) };

        let (_room, _member, members) = self.room_service.join_room(rid, uid, password).await
            .map_err(|e| e.to_string())?;

        // Get updated room and playback state
        let room = self.room_service.get_room(&rid).await
            .map_err(|e| e.to_string())?;
        let playback_state = self.room_service.get_playback_state(&rid).await.ok()
            .map(playback_state_to_proto);

        let proto_members: Vec<_> = members.into_iter().map(|m| {
            crate::proto::client::RoomMember {
                user_id: m.user_id.as_str().to_string(),
                username: m.username.clone(),
                role: m.role.to_string(),
                permissions: m.permissions.0,
            }
        }).collect();

        Ok(crate::proto::client::JoinRoomResponse {
            room: Some(room_to_proto_basic(&room)),
            members: proto_members,
            playback_state,
        })
    }

    pub async fn leave_room(
        &self,
        user_id: &str,
        req: crate::proto::client::LeaveRoomRequest,
    ) -> Result<crate::proto::client::LeaveRoomResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(req.room_id);

        self.room_service.leave_room(rid, uid).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::LeaveRoomResponse {
            success: true,
        })
    }

    pub async fn delete_room(
        &self,
        user_id: &str,
        req: crate::proto::client::DeleteRoomRequest,
    ) -> Result<crate::proto::client::DeleteRoomResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(req.room_id);

        self.room_service.delete_room(rid, uid).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::DeleteRoomResponse {
            success: true,
        })
    }

    pub async fn update_room_settings(
        &self,
        user_id: &str,
        room_id: &str,
        req: crate::proto::client::UpdateRoomSettingsRequest,
    ) -> Result<crate::proto::client::UpdateRoomSettingsResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(room_id.to_string());

        let settings = if !req.settings.is_empty() {
            Some(serde_json::from_slice(&req.settings).map_err(|e| e.to_string())?)
        } else {
            None
        };

        self.room_service.update_settings(&rid, uid, settings).await
            .map_err(|e| e.to_string())?;

        // Get updated room
        let room = self.room_service.get_room(&rid).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::UpdateRoomSettingsResponse {
            room: Some(room_to_proto_basic(&room)),
        })
    }

    // === Chat Operations ===

    pub async fn get_chat_history(
        &self,
        room_id: &str,
        req: crate::proto::client::GetChatHistoryRequest,
    ) -> Result<crate::proto::client::GetChatHistoryResponse, String> {
        let rid = RoomId::from_string(room_id.to_string());

        let messages = self.room_service.get_chat_history(&rid, None, req.limit).await
            .map_err(|e| e.to_string())?;

        // Collect unique user IDs to batch fetch usernames
        let user_ids: Vec<UserId> = messages
            .iter()
            .map(|m| m.user_id.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        // Batch fetch usernames
        let mut username_map = std::collections::HashMap::new();
        for user_id in user_ids {
            if let Ok(user) = self.user_service.get_user(&user_id).await {
                username_map.insert(user_id.to_string(), user.username);
            }
        }

        // Convert to proto format
        let proto_messages = messages
            .into_iter()
            .map(|m| {
                let username = username_map
                    .get(m.user_id.as_str())
                    .cloned()
                    .unwrap_or_else(|| format!("user_{}", m.user_id.as_str().to_string()));

                crate::proto::client::ChatMessageReceive {
                    id: m.id.clone(),
                    room_id: m.room_id.as_str().to_string(),
                    user_id: m.user_id.as_str().to_string(),
                    username,
                    content: m.content,
                    timestamp: m.created_at.timestamp(),
                }
            })
            .collect();

        Ok(crate::proto::client::GetChatHistoryResponse {
            messages: proto_messages,
        })
    }

    // === Media Operations ===

    pub async fn add_media(
        &self,
        user_id: &str,
        room_id: &str,
        req: crate::proto::client::AddMediaRequest,
    ) -> Result<crate::proto::client::AddMediaResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(room_id.to_string());

        let provider = if req.provider.is_empty() {
            ProviderType::DirectUrl
        } else {
            ProviderType::from_str(&req.provider)
                .unwrap_or(ProviderType::DirectUrl)
        };

        // Build source config from URL
        let source_config = serde_json::json!({
            "url": req.url
        });

        // Extract title from URL or use default
        let title = req.url.split('/').last().unwrap_or("Unknown").to_string();

        // For direct URL, provider_instance_name is empty
        let provider_instance_name = String::new();

        let media = self.room_service.add_media(
            rid,
            uid,
            provider_instance_name,
            source_config,
            title,
        ).await.map_err(|e| e.to_string())?;

        Ok(crate::proto::client::AddMediaResponse {
            media: Some(media_to_proto(&media)),
        })
    }

    pub async fn remove_media(
        &self,
        user_id: &str,
        room_id: &str,
        req: crate::proto::client::RemoveMediaRequest,
    ) -> Result<crate::proto::client::RemoveMediaResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(room_id.to_string());
        let mid = synctv_core::models::MediaId::from_string(req.media_id);

        self.room_service.remove_media(rid, uid, mid).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::RemoveMediaResponse {
            success: true,
        })
    }

    pub async fn get_playlist(
        &self,
        room_id: &str,
    ) -> Result<crate::proto::client::GetPlaylistResponse, String> {
        let rid = RoomId::from_string(room_id.to_string());
        let media_list = self.room_service.get_playlist(&rid).await
            .map_err(|e| e.to_string())?;

        let media: Vec<_> = media_list.into_iter().map(|m| media_to_proto(&m)).collect();

        Ok(crate::proto::client::GetPlaylistResponse {
            media,
        })
    }

    pub async fn swap_media(
        &self,
        user_id: &str,
        req: crate::proto::client::SwapMediaRequest,
    ) -> Result<crate::proto::client::SwapMediaResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(req.room_id.clone());
        let media_id1 = synctv_core::models::MediaId::from_string(req.media_id1.clone());
        let media_id2 = synctv_core::models::MediaId::from_string(req.media_id2.clone());

        self.room_service.swap_media(rid, uid, media_id1, media_id2).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::SwapMediaResponse {
            success: true,
        })
    }

    // === Playback Operations ===

    pub async fn play(
        &self,
        user_id: &str,
        room_id: &str,
        _req: crate::proto::client::PlayRequest,
    ) -> Result<crate::proto::client::PlayResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(room_id.to_string());

        let state = self.room_service.playback_service().set_playing(rid, uid, true).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::PlayResponse {
            playback_state: Some(playback_state_to_proto(&state)),
        })
    }

    pub async fn pause(
        &self,
        user_id: &str,
        room_id: &str,
    ) -> Result<crate::proto::client::PauseResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(room_id.to_string());

        let state = self.room_service.playback_service().set_playing(rid, uid, false).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::PauseResponse {
            playback_state: Some(playback_state_to_proto(&state)),
        })
    }

    pub async fn seek(
        &self,
        user_id: &str,
        room_id: &str,
        req: crate::proto::client::SeekRequest,
    ) -> Result<crate::proto::client::SeekResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(room_id.to_string());

        self.room_service.playback_service().seek(rid, uid, req.position).await
            .map_err(|e| e.to_string())?;

        let state = self.room_service.get_playback_state(&rid).await.ok();
        Ok(crate::proto::client::SeekResponse {
            playback_state: state.map(playback_state_to_proto),
        })
    }

    // === Member Operations ===

    pub async fn get_room_members(
        &self,
        room_id: &str,
    ) -> Result<crate::proto::client::GetRoomMembersResponse, String> {
        let rid = RoomId::from_string(room_id.to_string());
        let members = self.room_service.get_room_members(&rid).await
            .map_err(|e| e.to_string())?;

        let proto_members: Vec<_> = members.into_iter().map(|m| {
            crate::proto::client::RoomMember {
                user_id: m.user_id.as_str().to_string(),
                username: m.username.clone(),
                role: m.role.to_string(),
                permissions: m.permissions.0,
            }
        }).collect();

        Ok(crate::proto::client::GetRoomMembersResponse {
            members: proto_members,
        })
    }

    pub async fn update_member_permission(
        &self,
        user_id: &str,
        room_id: &str,
        req: crate::proto::client::UpdateMemberPermissionRequest,
    ) -> Result<crate::proto::client::UpdateMemberPermissionResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(room_id.to_string());
        let target_uid = UserId::from_string(req.user_id.clone());

        let permissions = synctv_core::models::PermissionBits(req.permissions);

        self.room_service.update_member_permission(rid, uid, target_uid, permissions).await
            .map_err(|e| e.to_string())?;

        // Get updated member
        let members = self.room_service.get_room_members(&rid).await
            .map_err(|e| e.to_string())?;
        let member = members.into_iter()
            .find(|m| m.user_id == target_uid)
            .ok_or_else(|| "Member not found".to_string())?;

        Ok(crate::proto::client::UpdateMemberPermissionResponse {
            member: Some(room_member_to_proto(&member)),
        })
    }

    pub async fn kick_member(
        &self,
        user_id: &str,
        room_id: &str,
        req: crate::proto::client::KickMemberRequest,
    ) -> Result<crate::proto::client::KickMemberResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(room_id.to_string());
        let target_uid = UserId::from_string(req.user_id.clone());

        self.room_service.kick_member(rid, uid, target_uid).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::KickMemberResponse {
            success: true,
        })
    }
}

// === Helper Functions ===

fn user_to_proto(user: &synctv_core::models::User) -> crate::proto::client::User {
    crate::proto::client::User {
        id: user.id.as_str().to_string(),
        username: user.username.clone(),
        email: user.email.clone().unwrap_or_default(),
        permissions: user.permissions.0,
        created_at: user.created_at.timestamp(),
    }
}

fn room_to_proto_basic(room: &synctv_core::models::Room) -> crate::proto::client::Room {
    crate::proto::client::Room {
        id: room.id.as_str().to_string(),
        name: room.name.clone(),
        created_by: room.created_by.as_str().to_string(),
        status: room.status.as_str().to_string(),
        settings: serde_json::to_vec(&room.settings).unwrap_or_default(),
        created_at: room.created_at.timestamp(),
        member_count: room.member_count,
    }
}

fn media_to_proto(media: &synctv_core::models::Media) -> crate::proto::client::Media {
    // Try to extract URL from source_config
    let url = media.source_config.get("url")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| String::new());

    crate::proto::client::Media {
        id: media.id.as_str().to_string(),
        room_id: media.room_id.as_str().to_string(),
        url,
        provider: media.source_provider.clone(),
        title: media.name.clone(),
        metadata: serde_json::to_vec(&media.metadata).unwrap_or_default(),
        position: media.position,
        added_at: media.added_at.timestamp(),
        added_by: media.creator_id.as_str().to_string(),
    }
}

fn playback_state_to_proto(state: &synctv_core::models::RoomPlaybackState) -> crate::proto::client::PlaybackState {
    crate::proto::client::PlaybackState {
        room_id: state.room_id.as_str().to_string(),
        playing_media_id: state.playing_media_id.map(|id| id.as_str().to_string()).unwrap_or_default(),
        position: state.position,
        speed: state.speed,
        is_playing: state.is_playing,
        updated_at: state.updated_at.timestamp(),
        version: state.version,
    }
}

fn room_member_to_proto(member: &synctv_core::models::RoomMemberWithUser) -> crate::proto::client::RoomMember {
    crate::proto::client::RoomMember {
        room_id: member.room_id.as_str().to_string(),
        user_id: member.user_id.as_str().to_string(),
        username: member.username.clone(),
        permissions: member.permissions.0,
        joined_at: member.joined_at.timestamp(),
        is_online: member.is_online,
    }
}
