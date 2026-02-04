//! Client API Implementation
//!
//! Unified implementation for all client API operations.
//! Used by both HTTP and gRPC handlers.

use std::sync::Arc;
use std::str::FromStr;
use synctv_core::models::{UserId, RoomId, ProviderType};
use synctv_core::service::{UserService, RoomService};
use synctv_cluster::sync::ConnectionManager;

/// Client API implementation
#[derive(Clone)]
pub struct ClientApiImpl {
    pub user_service: Arc<UserService>,
    pub room_service: Arc<RoomService>,
    pub connection_manager: Arc<ConnectionManager>,
}

impl ClientApiImpl {
    #[must_use] 
    pub const fn new(
        user_service: Arc<UserService>,
        room_service: Arc<RoomService>,
        connection_manager: Arc<ConnectionManager>,
    ) -> Self {
        Self {
            user_service,
            room_service,
            connection_manager,
        }
    }

    // === Auth Operations ===

    pub async fn register(
        &self,
        req: crate::proto::client::RegisterRequest,
    ) -> Result<crate::proto::client::RegisterResponse, String> {
        // Validate input
        if req.username.is_empty() {
            return Err("Username cannot be empty".to_string());
        }
        if req.password.len() < 6 {
            return Err("Password must be at least 6 characters".to_string());
        }

        let email = if req.email.is_empty() {
            Some(format!("{}@temp.local", req.username))
        } else {
            Some(req.email.clone())
        };

        // Register user (returns tuple: (User, access_token, refresh_token))
        let (user, access_token, refresh_token) = self
            .user_service
            .register(req.username, email, req.password)
            .await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::RegisterResponse {
            user: Some(user_to_proto(&user)),
            access_token,
            refresh_token,
        })
    }

    pub async fn login(
        &self,
        req: crate::proto::client::LoginRequest,
    ) -> Result<crate::proto::client::LoginResponse, String> {
        // Login user (returns tuple: (User, access_token, refresh_token))
        let (user, access_token, refresh_token) = self
            .user_service
            .login(req.username, req.password)
            .await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::LoginResponse {
            user: Some(user_to_proto(&user)),
            access_token,
            refresh_token,
        })
    }

    pub async fn refresh_token(
        &self,
        req: crate::proto::client::RefreshTokenRequest,
    ) -> Result<crate::proto::client::RefreshTokenResponse, String> {
        // Refresh tokens (returns tuple: (new_access_token, new_refresh_token))
        let (_access_token, _refresh_token) = self
            .user_service
            .refresh_token(req.refresh_token)
            .await
            .map_err(|e| e.to_string())?;

        // Note: The proto RefreshTokenResponse doesn't include user info
        // If needed, we could extract user_id from the new token and fetch user info
        Ok(crate::proto::client::RefreshTokenResponse {
            access_token: _access_token,
            refresh_token: _refresh_token,
        })
    }

    pub async fn logout(
        &self,
        _req: crate::proto::client::LogoutRequest,
    ) -> Result<crate::proto::client::LogoutResponse, String> {
        // Logout is typically handled client-side by deleting the token
        // If we had a token blacklist, we would add the token here
        Ok(crate::proto::client::LogoutResponse { success: true })
    }

    pub async fn get_profile(
        &self,
        user_id: &str,
    ) -> Result<crate::proto::client::GetProfileResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let user = self.user_service.get_user(&uid).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::GetProfileResponse {
            user: Some(user_to_proto(&user)),
        })
    }

    pub async fn set_username(
        &self,
        user_id: &str,
        req: crate::proto::client::SetUsernameRequest,
    ) -> Result<crate::proto::client::SetUsernameResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let user = self.user_service.get_user(&uid).await
            .map_err(|e| e.to_string())?;

        let updated_user = synctv_core::models::User {
            username: req.new_username,
            ..user
        };

        let result_user = self.user_service.update_user(&updated_user).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::SetUsernameResponse {
            user: Some(user_to_proto(&result_user)),
        })
    }

    pub async fn set_password(
        &self,
        user_id: &str,
        req: crate::proto::client::SetPasswordRequest,
    ) -> Result<crate::proto::client::SetPasswordResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        self.user_service.set_password(&uid, &req.new_password).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::SetPasswordResponse {
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

        let room_list: Vec<_> = rooms
            .into_iter()
            .map(|r| {
                let member_count = self
                    .connection_manager
                    .room_connection_count(&r.id)
                    .try_into()
                    .ok();
                room_to_proto_basic(&r, None, member_count)
            })
            .collect();

        Ok(crate::proto::client::ListRoomsResponse {
            rooms: room_list,
            total: total as i32,
        })
    }

    pub async fn get_joined_rooms(
        &self,
        user_id: &str,
    ) -> Result<crate::proto::client::ListParticipatedRoomsResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let (rooms, total) = self.room_service.list_joined_rooms_with_details(&uid, 1, 100).await
            .map_err(|e| e.to_string())?;

        let room_list: Vec<_> = rooms.into_iter().map(|(room, role, _status, _member_count)| {
            let role_str = match role {
                synctv_core::models::RoomRole::Creator => "creator",
                synctv_core::models::RoomRole::Admin => "admin",
                synctv_core::models::RoomRole::Member => "member",
                synctv_core::models::RoomRole::Guest => "guest",
            };
            let permissions = role.permissions().0;

            crate::proto::client::RoomWithRole {
                room: Some(room_to_proto_basic(
                    &room,
                    None,
                    self.connection_manager
                        .room_connection_count(&room.id)
                        .try_into()
                        .ok(),
                )),
                permissions,
                role: role_str.to_string(),
            }
        }).collect();

        Ok(crate::proto::client::ListParticipatedRoomsResponse {
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

        let settings = if req.settings.is_empty() {
            None
        } else {
            Some(serde_json::from_slice(&req.settings).map_err(|e| e.to_string())?)
        };

        let password = if req.password.is_empty() { None } else { Some(req.password) };

        let (room, _member) = self.room_service.create_room(req.name, uid, password, settings).await
            .map_err(|e| e.to_string())?;

        let member_count = self.connection_manager.room_connection_count(&room.id).try_into().ok();

        Ok(crate::proto::client::CreateRoomResponse {
            room: Some(room_to_proto_basic(&room, None, member_count)),
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
            .map(|s| playback_state_to_proto(&s));

        let member_count = self.connection_manager.room_connection_count(&rid).try_into().ok();

        Ok(crate::proto::client::GetRoomResponse {
            room: Some(room_to_proto_basic(&room, None, member_count)),
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

        let (_room, _member, members) = self.room_service.join_room(rid.clone(), uid, password).await
            .map_err(|e| e.to_string())?;

        // Get updated room and playback state
        let room = self.room_service.get_room(&rid).await
            .map_err(|e| e.to_string())?;
        let playback_state = self.room_service.get_playback_state(&rid).await.ok()
            .map(|s| playback_state_to_proto(&s));

        let proto_members: Vec<_> = members.into_iter()
            .map(room_member_to_proto)
            .collect();

        let member_count = self.connection_manager.room_connection_count(&rid).try_into().ok();

        Ok(crate::proto::client::JoinRoomResponse {
            room: Some(room_to_proto_basic(&room, None, member_count)),
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
        req: crate::proto::client::SetRoomSettingsRequest,
    ) -> Result<crate::proto::client::SetRoomSettingsResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(room_id.to_string());

        let settings = if req.settings.is_empty() {
            // Get current settings from room_settings table
            self.room_service.get_room_settings(&rid).await
                .map_err(|e| e.to_string())?
        } else {
            serde_json::from_slice::<synctv_core::models::RoomSettings>(&req.settings)
                .map_err(|e| e.to_string())?
        };

        self.room_service.set_settings(rid.clone(), uid, settings).await
            .map_err(|e| e.to_string())?;

        // Get updated room
        let room = self.room_service.get_room(&rid).await
            .map_err(|e| e.to_string())?;

        let member_count = self.connection_manager.room_connection_count(&rid).try_into().ok();

        Ok(crate::proto::client::SetRoomSettingsResponse {
            room: Some(room_to_proto_basic(&room, None, member_count)),
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
                    .unwrap_or_else(|| format!("user_{}", m.user_id.as_str()));

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

        let _provider = if req.provider.is_empty() {
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
        let title = req.url.split('/').next_back().unwrap_or("Unknown").to_string();

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
        let total = media.len() as i32;

        // TODO: Get actual playlist info
        let playlist = Some(crate::proto::client::Playlist {
            id: String::new(),
            room_id: rid.as_str().to_string(),
            name: String::new(),
            parent_id: String::new(),
            position: 0,
            is_folder: false,
            is_dynamic: false,
            item_count: total,
            created_at: 0,
            updated_at: 0,
        });

        Ok(crate::proto::client::GetPlaylistResponse {
            playlist,
            media,
            total,
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

        self.room_service.playback_service().seek(rid.clone(), uid, req.position).await
            .map_err(|e| e.to_string())?;

        let state = self.room_service.get_playback_state(&rid).await.ok();
        Ok(crate::proto::client::SeekResponse {
            playback_state: state.map(|s| playback_state_to_proto(&s)),
        })
    }

    pub async fn change_speed(
        &self,
        user_id: &str,
        room_id: &str,
        req: crate::proto::client::ChangeSpeedRequest,
    ) -> Result<crate::proto::client::ChangeSpeedResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(room_id.to_string());

        self.room_service.playback_service().change_speed(rid.clone(), uid, req.speed).await
            .map_err(|e| e.to_string())?;

        let state = self.room_service.get_playback_state(&rid).await.ok();
        Ok(crate::proto::client::ChangeSpeedResponse {
            playback_state: state.map(|s| playback_state_to_proto(&s)),
        })
    }

    pub async fn switch_media(
        &self,
        user_id: &str,
        room_id: &str,
        req: crate::proto::client::SwitchMediaRequest,
    ) -> Result<crate::proto::client::SwitchMediaResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(room_id.to_string());
        let media_id = synctv_core::models::MediaId::from_string(req.media_id);

        self.room_service.playback_service().switch_media(rid.clone(), uid, media_id).await
            .map_err(|e| e.to_string())?;

        let state = self.room_service.get_playback_state(&rid).await.ok();
        Ok(crate::proto::client::SwitchMediaResponse {
            playback_state: state.map(|s| playback_state_to_proto(&s)),
        })
    }

    pub async fn get_playback_state(
        &self,
        room_id: &str,
        _req: crate::proto::client::GetPlaybackStateRequest,
    ) -> Result<crate::proto::client::GetPlaybackStateResponse, String> {
        let rid = RoomId::from_string(room_id.to_string());

        let state = self.room_service.get_playback_state(&rid).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::GetPlaybackStateResponse {
            playback_state: Some(playback_state_to_proto(&state)),
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

        let proto_members: Vec<_> = members.into_iter()
            .map(room_member_to_proto)
            .collect();

        Ok(crate::proto::client::GetRoomMembersResponse {
            members: proto_members,
        })
    }

    pub async fn update_member_permission(
        &self,
        user_id: &str,
        room_id: &str,
        req: crate::proto::client::SetMemberPermissionRequest,
    ) -> Result<crate::proto::client::SetMemberPermissionResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(room_id.to_string());
        let target_uid = UserId::from_string(req.user_id.clone());

        // Handle role update if provided
        if !req.role.is_empty() {
            let new_role = synctv_core::models::RoomRole::from_str(&req.role)?;
            // Update the member role
            self.room_service.member_service().set_member_role(
                rid.clone(),
                uid.clone(),
                target_uid.clone(),
                new_role,
            ).await.map_err(|e| e.to_string())?;
        }

        // Determine which permission set to use based on current role
        // (This is a simplified implementation - proper logic would check the member's current role)
        let use_admin_perms = req.admin_added_permissions > 0 || req.admin_removed_permissions > 0;

        let added = if use_admin_perms {
            req.admin_added_permissions
        } else {
            req.added_permissions
        };

        let removed = if use_admin_perms {
            req.admin_removed_permissions
        } else {
            req.removed_permissions
        };

        self.room_service.set_member_permission(
            rid.clone(),
            uid,
            target_uid.clone(),
            added,
            removed,
        ).await
            .map_err(|e| e.to_string())?;

        // Get updated member
        let members = self.room_service.get_room_members(&rid).await
            .map_err(|e| e.to_string())?;
        let member = members.into_iter()
            .find(|m| m.user_id == target_uid)
            .ok_or_else(|| "Member not found".to_string())?;

        Ok(crate::proto::client::SetMemberPermissionResponse {
            member: Some(room_member_to_proto(member)),
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
    let role_str = match user.role {
        synctv_core::models::UserRole::Root => "root",
        synctv_core::models::UserRole::Admin => "admin",
        synctv_core::models::UserRole::User => "user",
    };

    let status_str = match user.status {
        synctv_core::models::UserStatus::Active => "active",
        synctv_core::models::UserStatus::Pending => "pending",
        synctv_core::models::UserStatus::Banned => "banned",
    };

    crate::proto::client::User {
        id: user.id.as_str().to_string(),
        username: user.username.clone(),
        email: user.email.clone().unwrap_or_default(),
        role: role_str.to_string(),
        status: status_str.to_string(),
        created_at: user.created_at.timestamp(),
    }
}

fn room_to_proto_basic(
    room: &synctv_core::models::Room,
    settings: Option<&synctv_core::models::RoomSettings>,
    member_count: Option<i32>,
) -> crate::proto::client::Room {
    let room_settings = settings.cloned().unwrap_or_default();
    crate::proto::client::Room {
        id: room.id.as_str().to_string(),
        name: room.name.clone(),
        created_by: room.created_by.as_str().to_string(),
        status: room.status.as_str().to_string(),
        settings: serde_json::to_vec(&room_settings).unwrap_or_default(),
        created_at: room.created_at.timestamp(),
        member_count: member_count.unwrap_or(0),
    }
}

#[must_use]
pub fn media_to_proto(media: &synctv_core::models::Media) -> crate::proto::client::Media {
    // Try to extract URL from source_config
    let url = media.source_config.get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Get metadata from PlaybackResult if available (for direct URLs)
    let metadata_bytes = if media.is_direct() {
        media
            .get_playback_result()
            .map(|pb| serde_json::to_vec(&pb.metadata).unwrap_or_default())
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    crate::proto::client::Media {
        id: media.id.as_str().to_string(),
        room_id: media.room_id.as_str().to_string(),
        url: url.to_string(),
        provider: media.source_provider.clone(),
        title: media.name.clone(),
        metadata: metadata_bytes,
        position: media.position,
        added_at: media.added_at.timestamp(),
        added_by: media.creator_id.as_str().to_string(),
        provider_instance_name: media.provider_instance_name.clone().unwrap_or_default(),
        source_config: serde_json::to_vec(&media.source_config).unwrap_or_default(),
    }
}

fn playback_state_to_proto(state: &synctv_core::models::RoomPlaybackState) -> crate::proto::client::PlaybackState {
    crate::proto::client::PlaybackState {
        room_id: state.room_id.as_str().to_string(),
        playing_media_id: state.playing_media_id.as_ref().map(|id| id.as_str().to_string()).unwrap_or_default(),
        position: state.position,
        speed: state.speed,
        is_playing: state.is_playing,
        updated_at: state.updated_at.timestamp(),
        version: state.version,
    }
}

fn room_member_to_proto(member: synctv_core::models::RoomMemberWithUser) -> crate::proto::client::RoomMember {
    let role_str = match member.role {
        synctv_core::models::RoomRole::Creator => "creator",
        synctv_core::models::RoomRole::Admin => "admin",
        synctv_core::models::RoomRole::Member => "member",
        synctv_core::models::RoomRole::Guest => "guest",
    };

    crate::proto::client::RoomMember {
        room_id: member.room_id.as_str().to_string(),
        user_id: member.user_id.as_str().to_string(),
        username: member.username.clone(),
        role: role_str.to_string(),
        permissions: member.effective_permissions(synctv_core::models::PermissionBits::empty()).0,
        added_permissions: member.added_permissions,
        removed_permissions: member.removed_permissions,
        admin_added_permissions: member.admin_added_permissions,
        admin_removed_permissions: member.admin_removed_permissions,
        joined_at: member.joined_at.timestamp(),
        is_online: member.is_online,
    }
}
