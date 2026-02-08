//! Client API Implementation
//!
//! Unified implementation for all client API operations.
//! Used by both HTTP and gRPC handlers.

use std::collections::HashMap;
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
    pub config: Arc<synctv_core::Config>,
    pub sfu_manager: Option<Arc<synctv_sfu::SfuManager>>,
    pub publish_key_service: Option<Arc<synctv_core::service::PublishKeyService>>,
}

impl ClientApiImpl {
    #[must_use]
    pub const fn new(
        user_service: Arc<UserService>,
        room_service: Arc<RoomService>,
        connection_manager: Arc<ConnectionManager>,
        config: Arc<synctv_core::Config>,
        sfu_manager: Option<Arc<synctv_sfu::SfuManager>>,
        publish_key_service: Option<Arc<synctv_core::service::PublishKeyService>>,
    ) -> Self {
        Self {
            user_service,
            room_service,
            connection_manager,
            config,
            sfu_manager,
            publish_key_service,
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
        req: crate::proto::client::ListRoomsRequest,
    ) -> Result<crate::proto::client::ListRoomsResponse, String> {
        let mut query = synctv_core::models::RoomListQuery {
            page: req.page,
            page_size: req.page_size,
            ..Default::default()
        };
        if !req.search.is_empty() {
            query.search = Some(req.search);
        }
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

        let (room, _member) = self.room_service.create_room(req.name, req.description, uid, password, settings).await
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
        req: crate::proto::client::UpdateRoomSettingsRequest,
    ) -> Result<crate::proto::client::UpdateRoomSettingsResponse, String> {
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

        Ok(crate::proto::client::UpdateRoomSettingsResponse {
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
                    position: None, // History messages don't show as danmaku
                    color: None,
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

    /// Bulk remove multiple media items
    pub async fn remove_media_batch(
        &self,
        user_id: &str,
        room_id: &str,
        media_ids: Vec<String>,
    ) -> Result<usize, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(room_id.to_string());
        let mids: Vec<synctv_core::models::MediaId> = media_ids
            .into_iter()
            .map(synctv_core::models::MediaId::from_string)
            .collect();

        self.room_service
            .media_service()
            .remove_media_batch(rid, uid, mids)
            .await
            .map_err(|e| e.to_string())
    }

    /// Bulk reorder multiple media items
    pub async fn reorder_media_batch(
        &self,
        user_id: &str,
        room_id: &str,
        updates: Vec<(String, i32)>,
    ) -> Result<(), String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(room_id.to_string());
        let updates_converted: Vec<(synctv_core::models::MediaId, i32)> = updates
            .into_iter()
            .map(|(id, pos)| (synctv_core::models::MediaId::from_string(id), pos))
            .collect();

        self.room_service
            .media_service()
            .reorder_media_batch(rid, uid, updates_converted)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn get_playlist(
        &self,
        room_id: &str,
    ) -> Result<crate::proto::client::ListPlaylistResponse, String> {
        let rid = RoomId::from_string(room_id.to_string());
        let media_list = self.room_service.get_playlist(&rid).await
            .map_err(|e| e.to_string())?;

        let media: Vec<_> = media_list.into_iter().map(|m| media_to_proto(&m)).collect();
        let total = media.len() as i32;

        let playlist = match self.room_service.playlist_service().get_root_playlist(&rid).await {
            Ok(pl) => Some(crate::proto::client::Playlist {
                id: pl.id.as_str().to_string(),
                room_id: pl.room_id.as_str().to_string(),
                name: pl.name.clone(),
                parent_id: pl.parent_id.as_ref().map_or(String::new(), |p| p.as_str().to_string()),
                position: pl.position,
                is_folder: true,
                is_dynamic: pl.is_dynamic(),
                item_count: total,
                created_at: pl.created_at.timestamp(),
                updated_at: pl.updated_at.timestamp(),
            }),
            Err(_) => Some(crate::proto::client::Playlist {
                id: String::new(),
                room_id: rid.as_str().to_string(),
                name: String::new(),
                parent_id: String::new(),
                position: 0,
                is_folder: true,
                is_dynamic: false,
                item_count: total,
                created_at: 0,
                updated_at: 0,
            }),
        };

        Ok(crate::proto::client::ListPlaylistResponse {
            playlist,
            media,
            total,
        })
    }

    pub async fn list_playlist_items(
        &self,
        user_id: &str,
        req: crate::proto::client::ListPlaylistItemsRequest,
    ) -> Result<crate::proto::client::ListPlaylistItemsResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(req.room_id.clone());
        let playlist_id = synctv_core::models::PlaylistId::from_string(req.playlist_id.clone());

        let relative_path = if req.relative_path.is_empty() {
            None
        } else {
            Some(req.relative_path.as_str())
        };

        let page = req.page.max(0) as usize;
        let page_size = req.page_size.clamp(1, 100) as usize;

        let items = self
            .room_service
            .media_service()
            .list_dynamic_playlist_items(rid, uid, &playlist_id, relative_path, page, page_size)
            .await
            .map_err(|e| e.to_string())?;

        // Convert DirectoryItem to proto DirectoryItem
        let proto_items: Vec<_> = items
            .into_iter()
            .map(|item| {
                use synctv_core::provider::ItemType;
                let item_type = match item.item_type {
                    ItemType::Video => crate::proto::client::ItemType::Video as i32,
                    ItemType::Audio => crate::proto::client::ItemType::Audio as i32,
                    ItemType::Folder => crate::proto::client::ItemType::Folder as i32,
                    ItemType::Live => crate::proto::client::ItemType::Live as i32,
                    ItemType::File => crate::proto::client::ItemType::File as i32,
                };

                crate::proto::client::DirectoryItem {
                    name: item.name,
                    item_type,
                    path: item.path,
                    size: item.size.map(|s| s as i64),
                    thumbnail: item.thumbnail,
                    modified_at: item.modified_at,
                }
            })
            .collect();

        let total = proto_items.len() as i32;

        Ok(crate::proto::client::ListPlaylistItemsResponse {
            items: proto_items,
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

    pub async fn set_playback_speed(
        &self,
        user_id: &str,
        room_id: &str,
        req: crate::proto::client::SetPlaybackSpeedRequest,
    ) -> Result<crate::proto::client::SetPlaybackSpeedResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(room_id.to_string());

        self.room_service.playback_service().change_speed(rid.clone(), uid, req.speed).await
            .map_err(|e| e.to_string())?;

        let state = self.room_service.get_playback_state(&rid).await.ok();
        Ok(crate::proto::client::SetPlaybackSpeedResponse {
            playback_state: state.map(|s| playback_state_to_proto(&s)),
        })
    }

    // set_current_media - Set which media to play (previously set_playing)
    pub async fn set_current_media(
        &self,
        user_id: &str,
        room_id: &str,
        req: crate::proto::client::SetCurrentMediaRequest,
    ) -> Result<crate::proto::client::SetCurrentMediaResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(room_id.to_string());

        // If media_id is provided, switch to that media
        if !req.media_id.is_empty() {
            let media_id = synctv_core::models::MediaId::from_string(req.media_id);
            self.room_service.playback_service().switch_media(rid.clone(), uid, media_id).await
                .map_err(|e| e.to_string())?;
        }

        // Get the current root playlist and its item count
        let playlist = self.room_service.playlist_service().get_root_playlist(&rid).await
            .map_err(|e| e.to_string())?;
        let item_count = self.room_service.media_service().count_playlist_media(&playlist.id).await
            .map_err(|e| e.to_string())? as i32;

        // Get the currently playing media
        let playing_media = self.room_service.get_playing_media(&rid).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::SetCurrentMediaResponse {
            playlist: Some(playlist_to_proto(&playlist, item_count)),
            playing_media: playing_media.map(|m| media_to_proto(&m)),
        })
    }

    // Note: switch_media removed - use set_current_media instead

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

    // === Live Streaming Operations ===

    pub async fn create_publish_key(
        &self,
        user_id: &str,
        room_id: &str,
        req: crate::proto::client::CreatePublishKeyRequest,
    ) -> Result<crate::proto::client::CreatePublishKeyResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(room_id.to_string());

        // Validate media ID
        if req.id.is_empty() {
            return Err("Media ID is required".to_string());
        }
        let media_id = synctv_core::models::MediaId::from_string(req.id.clone());

        // Check room exists
        let _room = self.room_service.get_room(&rid).await
            .map_err(|e| match e {
                synctv_core::Error::NotFound(msg) => format!("Room not found: {msg}"),
                _ => format!("Failed to get room: {e}"),
            })?;

        // Check permission to start live stream
        self.room_service
            .check_permission(&rid, &uid, synctv_core::models::PermissionBits::START_LIVE)
            .await
            .map_err(|e| format!("Permission denied: {e}"))?;

        // Get publish key service
        let publish_key_service = self.publish_key_service.as_ref()
            .ok_or_else(|| "Publish key service not configured".to_string())?;

        // Generate publish key
        let publish_key = publish_key_service
            .generate_publish_key(rid.clone(), media_id.clone(), uid.clone())
            .await
            .map_err(|e| format!("Failed to generate publish key: {e}"))?;

        // Construct RTMP URL and stream key
        let rtmp_url = "rtmp://localhost:1935/live".to_string();
        let stream_key = format!("{}/{}", rid.as_str(), media_id.as_str());

        tracing::info!(
            room_id = %rid.as_str(),
            media_id = %media_id.as_str(),
            user_id = %uid.as_str(),
            expires_at = publish_key.expires_at,
            "Generated publish key for live streaming"
        );

        Ok(crate::proto::client::CreatePublishKeyResponse {
            publish_key: publish_key.token,
            rtmp_url,
            stream_key,
            expires_at: publish_key.expires_at,
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

    pub async fn update_member_permissions(
        &self,
        user_id: &str,
        room_id: &str,
        req: crate::proto::client::UpdateMemberPermissionsRequest,
    ) -> Result<crate::proto::client::UpdateMemberPermissionsResponse, String> {
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

        Ok(crate::proto::client::UpdateMemberPermissionsResponse {
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

    pub async fn ban_member(
        &self,
        user_id: &str,
        room_id: &str,
        req: crate::proto::client::BanMemberRequest,
    ) -> Result<crate::proto::client::BanMemberResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(room_id.to_string());
        let target_uid = UserId::from_string(req.user_id.clone());
        let reason = if req.reason.is_empty() { None } else { Some(req.reason) };

        self.room_service.member_service()
            .ban_member(rid, uid, target_uid, reason)
            .await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::BanMemberResponse { success: true })
    }

    pub async fn unban_member(
        &self,
        user_id: &str,
        room_id: &str,
        req: crate::proto::client::UnbanMemberRequest,
    ) -> Result<crate::proto::client::UnbanMemberResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let rid = RoomId::from_string(room_id.to_string());
        let target_uid = UserId::from_string(req.user_id.clone());

        self.room_service.member_service()
            .unban_member(rid, uid, target_uid)
            .await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::UnbanMemberResponse { success: true })
    }

    // === Movie Info ===

    /// Get movie info for a media item (resolves provider playback + proxy/direct decision)
    #[allow(clippy::too_many_arguments)]
    pub async fn get_movie_info(
        &self,
        user_id: &str,
        room_id: &str,
        media_id: &str,
        bilibili_provider: &synctv_core::provider::BilibiliProvider,
        alist_provider: &synctv_core::provider::AlistProvider,
        emby_provider: &synctv_core::provider::EmbyProvider,
        settings_registry: Option<&synctv_core::service::SettingsRegistry>,
    ) -> Result<crate::proto::client::GetMovieInfoResponse, String> {
        use synctv_core::provider::{MediaProvider, ProviderContext};

        let rid = RoomId::from_string(room_id.to_string());
        let mid = synctv_core::models::MediaId::from_string(media_id.to_string());

        // 1. Get media from playlist
        let playlist = self
            .room_service
            .get_playlist(&rid)
            .await
            .map_err(|e| format!("Failed to get playlist: {e}"))?;

        let media = playlist
            .iter()
            .find(|m| m.id == mid)
            .ok_or_else(|| "Media not found in playlist".to_string())?;

        // 2. Determine provider
        let provider: &dyn MediaProvider = match media.source_provider.as_str() {
            "bilibili" => bilibili_provider as &dyn MediaProvider,
            "alist" => alist_provider as &dyn MediaProvider,
            "emby" => emby_provider as &dyn MediaProvider,
            other => return Err(format!("Unknown provider: {other}")),
        };

        // 3. Call generate_playback
        let ctx = ProviderContext::new("synctv")
            .with_user_id(user_id)
            .with_room_id(room_id);

        let result = provider
            .generate_playback(&ctx, &media.source_config)
            .await
            .map_err(|e| format!("generate_playback failed: {e}"))?;

        // 4. Check movie_proxy setting
        let movie_proxy = settings_registry
            .map(|r| r.to_public_settings().movie_proxy)
            .unwrap_or(true);

        // 5. Build MovieInfo
        let is_live = result
            .metadata
            .get("is_live")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let duration = result
            .metadata
            .get("duration")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let subtitles: Vec<crate::proto::client::MovieSubtitle> = result
            .playback_infos
            .values()
            .flat_map(|pi| &pi.subtitles)
            .map(|s| crate::proto::client::MovieSubtitle {
                name: s.name.clone(),
                language: s.language.clone(),
                url: if movie_proxy {
                    format!(
                        "/api/providers/{}/proxy/{room_id}/{media_id}/subtitle/{}",
                        media.source_provider,
                        url::form_urlencoded::byte_serialize(s.name.as_bytes())
                            .collect::<String>()
                    )
                } else {
                    s.url.clone()
                },
                format: s.format.clone(),
            })
            .collect();

        // For DASH providers (bilibili Video/Pgc): generate MPD URLs
        if result.dash.is_some() {
            let (url, media_type, headers) = if movie_proxy {
                (
                    format!(
                        "/api/providers/{}/proxy/{room_id}/{media_id}/mpd",
                        media.source_provider
                    ),
                    "mpd".to_string(),
                    HashMap::new(),
                )
            } else {
                // Direct mode: client gets CDN URLs in MPD
                // For direct, we still serve an MPD but with CDN BaseURLs
                (
                    format!(
                        "/api/providers/{}/proxy/{room_id}/{media_id}/mpd?direct=1",
                        media.source_provider
                    ),
                    "mpd".to_string(),
                    result
                        .playback_infos
                        .get("dash")
                        .map(|pi| pi.headers.clone())
                        .unwrap_or_default(),
                )
            };

            let mut more_sources = Vec::new();
            if result.hevc_dash.is_some() {
                let hevc_url = if movie_proxy {
                    format!(
                        "/api/providers/{}/proxy/{room_id}/{media_id}/mpd?codec=hevc",
                        media.source_provider
                    )
                } else {
                    format!(
                        "/api/providers/{}/proxy/{room_id}/{media_id}/mpd?codec=hevc&direct=1",
                        media.source_provider
                    )
                };
                more_sources.push(crate::proto::client::MovieSource {
                    name: "HEVC".to_string(),
                    r#type: "mpd".to_string(),
                    url: hevc_url,
                    headers: HashMap::new(),
                });
            }

            return Ok(crate::proto::client::GetMovieInfoResponse {
                movie: Some(crate::proto::client::MovieInfo {
                    r#type: media_type,
                    url,
                    headers,
                    more_sources,
                    subtitles,
                    is_live,
                    duration,
                }),
            });
        }

        // For HLS/direct providers: use first URL from default playback mode
        let default_info = result.playback_infos.get(&result.default_mode);
        let (url, media_type, headers) = if let Some(info) = default_info {
            let first_url = info.urls.first().cloned().unwrap_or_default();
            if movie_proxy && !first_url.is_empty() {
                (
                    format!(
                        "/api/providers/{}/proxy/{room_id}/{media_id}",
                        media.source_provider
                    ),
                    info.format.clone(),
                    HashMap::new(),
                )
            } else {
                (first_url, info.format.clone(), info.headers.clone())
            }
        } else {
            (String::new(), "unknown".to_string(), HashMap::new())
        };

        Ok(crate::proto::client::GetMovieInfoResponse {
            movie: Some(crate::proto::client::MovieInfo {
                r#type: media_type,
                url,
                headers,
                more_sources: Vec::new(),
                subtitles,
                is_live,
                duration,
            }),
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
        email_verified: user.email_verified,
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
        description: room.description.clone(),
        created_by: room.created_by.as_str().to_string(),
        status: room.status.as_str().to_string(),
        settings: serde_json::to_vec(&room_settings).unwrap_or_default(),
        created_at: room.created_at.timestamp(),
        member_count: member_count.unwrap_or(0),
        updated_at: room.updated_at.timestamp(),
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

fn playlist_to_proto(playlist: &synctv_core::models::Playlist, item_count: i32) -> crate::proto::client::Playlist {
    crate::proto::client::Playlist {
        id: playlist.id.as_str().to_string(),
        room_id: playlist.room_id.as_str().to_string(),
        name: playlist.name.clone(),
        parent_id: playlist.parent_id.as_ref().map(|id| id.as_str().to_string()).unwrap_or_default(),
        position: playlist.position,
        is_folder: playlist.parent_id.is_none() || playlist.source_provider.is_some(),
        is_dynamic: playlist.is_dynamic(),
        item_count,
        created_at: playlist.created_at.timestamp(),
        updated_at: playlist.updated_at.timestamp(),
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

impl ClientApiImpl {
    // === WebRTC Operations ===

    /// Get ICE servers configuration for WebRTC
    pub async fn get_ice_servers(
        &self,
        _room_id: &RoomId,
        user_id: &UserId,
    ) -> Result<crate::proto::client::GetIceServersResponse, anyhow::Error> {
        use crate::proto::client::{IceServer, GetIceServersResponse};
        use synctv_core::config::TurnMode;

        let webrtc_config = &self.config.webrtc;
        let mut servers = Vec::new();

        // Add built-in STUN server if enabled
        if webrtc_config.enable_builtin_stun {
            let stun_url = format!(
                "stun:{}:{}",
                self.config.server.host,
                webrtc_config.builtin_stun_port
            );
            servers.push(IceServer {
                urls: vec![stun_url],
                username: None,
                credential: None,
            });
        }

        // Add external STUN servers
        for url in &webrtc_config.external_stun_servers {
            servers.push(IceServer {
                urls: vec![url.clone()],
                username: None,
                credential: None,
            });
        }

        // Add TURN server based on configured mode
        match webrtc_config.turn_mode {
            TurnMode::Builtin => {
                if webrtc_config.enable_builtin_turn {
                    // Use built-in TURN server
                    let turn_url = format!(
                        "turn:{}:{}",
                        self.config.server.host,
                        webrtc_config.builtin_turn_port
                    );

                    // Get static secret for credential generation
                    if let Some(turn_secret) = &webrtc_config.external_turn_static_secret {
                        let turn_config = synctv_core::service::TurnConfig {
                            server_url: turn_url.clone(),
                            static_secret: turn_secret.clone(),
                            credential_ttl: std::time::Duration::from_secs(webrtc_config.turn_credential_ttl),
                            use_tls: false,
                        };
                        let turn_service = synctv_core::service::TurnCredentialService::new(turn_config);

                        // Generate time-limited credentials
                        let credential = turn_service.generate_credential(user_id.as_str())?;

                        servers.push(IceServer {
                            urls: vec![turn_url],
                            username: Some(credential.username),
                            credential: Some(credential.password),
                        });
                    }
                }
            }
            TurnMode::External => {
                // Use external TURN server (coturn)
                if let (Some(turn_url), Some(turn_secret)) = (
                    &webrtc_config.external_turn_server_url,
                    &webrtc_config.external_turn_static_secret,
                ) {
                    let turn_config = synctv_core::service::TurnConfig {
                        server_url: turn_url.clone(),
                        static_secret: turn_secret.clone(),
                        credential_ttl: std::time::Duration::from_secs(webrtc_config.turn_credential_ttl),
                        use_tls: false,
                    };
                    let turn_service = synctv_core::service::TurnCredentialService::new(turn_config);

                    // Generate time-limited credentials
                    let credential = turn_service.generate_credential(user_id.as_str())?;

                    // Get all TURN URLs (including TLS variant if enabled)
                    let urls = turn_service.get_urls();

                    servers.push(IceServer {
                        urls,
                        username: Some(credential.username),
                        credential: Some(credential.password),
                    });
                }
            }
            TurnMode::Disabled => {
                // TURN disabled - rely on STUN only for NAT traversal
                // This may result in ~85-90% connection success rate instead of ~99%
            }
        }

        Ok(GetIceServersResponse { servers })
    }

    /// Get network quality stats for peers in a room
    pub async fn get_network_quality(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
    ) -> Result<crate::proto::client::GetNetworkQualityResponse, anyhow::Error> {
        use crate::proto::client::GetNetworkQualityResponse;

        let sfu_manager = match &self.sfu_manager {
            Some(mgr) => mgr,
            None => {
                tracing::debug!(
                    room_id = %room_id,
                    user_id = %user_id,
                    "Network quality requested but SFU manager not enabled"
                );
                return Ok(GetNetworkQualityResponse { peers: vec![] });
            }
        };

        let stats = sfu_manager.get_room_network_quality(
            &synctv_sfu::RoomId::from(room_id.as_str()),
        )?;

        let peers = stats
            .into_iter()
            .map(|(peer_id, ns)| network_stats_to_proto(peer_id, ns))
            .collect();

        Ok(GetNetworkQualityResponse { peers })
    }
}

/// Convert SFU `NetworkStats` to proto `PeerNetworkQuality`
pub fn network_stats_to_proto(
    peer_id: String,
    ns: synctv_sfu::NetworkStats,
) -> crate::proto::client::PeerNetworkQuality {
    let quality_action = match ns.quality_action {
        synctv_sfu::QualityAction::None => "none",
        synctv_sfu::QualityAction::ReduceQuality => "reduce_quality",
        synctv_sfu::QualityAction::ReduceFramerate => "reduce_framerate",
        synctv_sfu::QualityAction::AudioOnly => "audio_only",
    };
    crate::proto::client::PeerNetworkQuality {
        peer_id,
        rtt_ms: ns.rtt_ms,
        packet_loss_rate: ns.packet_loss_rate,
        jitter_ms: ns.jitter_ms,
        available_bandwidth_kbps: ns.available_bandwidth_kbps,
        quality_score: u32::from(ns.quality_score),
        quality_action: quality_action.to_string(),
    }
}
