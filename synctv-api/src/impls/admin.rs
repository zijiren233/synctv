//! Admin API Implementation
//!
//! Unified implementation for all admin API operations.
//! Used by both HTTP and gRPC handlers.

use std::sync::Arc;
use std::str::FromStr;
use synctv_core::models::{UserId, RoomId, UserRole, UserStatus};
use synctv_core::service::{RoomService, UserService, SettingsService, EmailService, RemoteProviderManager, SettingsRegistry};
use synctv_cluster::sync::ConnectionManager;

/// Admin API implementation
#[derive(Clone)]
pub struct AdminApiImpl {
    pub room_service: Arc<RoomService>,
    pub user_service: Arc<UserService>,
    pub settings_service: Arc<SettingsService>,
    pub settings_registry: Option<Arc<SettingsRegistry>>,
    pub email_service: Arc<EmailService>,
    pub connection_manager: Arc<ConnectionManager>,
    pub provider_instance_manager: Arc<RemoteProviderManager>,
}

impl AdminApiImpl {
    #[must_use]
    pub fn new(
        room_service: Arc<RoomService>,
        user_service: Arc<UserService>,
        settings_service: Arc<SettingsService>,
        settings_registry: Option<Arc<SettingsRegistry>>,
        email_service: Arc<EmailService>,
        connection_manager: Arc<ConnectionManager>,
        provider_instance_manager: Arc<RemoteProviderManager>,
    ) -> Self {
        Self {
            room_service,
            user_service,
            settings_service,
            settings_registry,
            email_service,
            connection_manager,
            provider_instance_manager,
        }
    }

    // === Room Management ===

    pub async fn list_rooms(
        &self,
        req: crate::proto::admin::ListRoomsRequest,
    ) -> Result<crate::proto::admin::ListRoomsResponse, String> {
        let page = if req.page > 0 { req.page } else { 1 };
        let page_size = if req.page_size > 0 { req.page_size } else { 50 };

        let query = synctv_core::models::RoomListQuery {
            page,
            page_size,
            ..Default::default()
        };

        let (rooms, total) = self.room_service.list_rooms(&query).await
            .map_err(|e| e.to_string())?;

        let room_list: Vec<_> = rooms
            .into_iter()
            .map(|r| {
                // Get online member count from connection manager
                let member_count = self
                    .connection_manager
                    .room_connection_count(&r.id)
                    .try_into()
                    .ok();
                admin_room_to_proto(&r, None, member_count)
            })
            .collect();

        Ok(crate::proto::admin::ListRoomsResponse {
            rooms: room_list,
            total: total as i32,
        })
    }

    pub async fn get_room(
        &self,
        req: crate::proto::admin::GetRoomRequest,
    ) -> Result<crate::proto::admin::GetRoomResponse, String> {
        let rid = RoomId::from_string(req.room_id);
        let room = self.room_service.get_room(&rid).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::GetRoomResponse {
            room: Some(admin_room_to_proto(
                &room,
                None,
                self.connection_manager
                    .room_connection_count(&room.id)
                    .try_into()
                    .ok(),
            )),
        })
    }

    pub async fn delete_room(
        &self,
        req: crate::proto::admin::DeleteRoomRequest,
    ) -> Result<crate::proto::admin::DeleteRoomResponse, String> {
        let rid = RoomId::from_string(req.room_id);
        // Use a system admin user for admin operations
        let admin_uid = UserId::from_string("system".to_string());

        self.room_service.delete_room(rid, admin_uid).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::DeleteRoomResponse {
            success: true,
        })
    }

    pub async fn update_room_password(
        &self,
        req: crate::proto::admin::UpdateRoomPasswordRequest,
    ) -> Result<crate::proto::admin::UpdateRoomPasswordResponse, String> {
        // For admin operations, use a system user ID (in real implementation, you'd get this from auth context)
        use synctv_core::models::UserId;
        let admin_user_id = UserId::new(); // System user

        self.room_service.set_room_password(req, &admin_user_id).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::UpdateRoomPasswordResponse {
            success: true,
        })
    }

    pub async fn get_room_members(
        &self,
        req: crate::proto::admin::GetRoomMembersRequest,
    ) -> Result<crate::proto::admin::GetRoomMembersResponse, String> {
        let rid = RoomId::from_string(req.room_id);
        let members = self.room_service.get_room_members(&rid).await
            .map_err(|e| e.to_string())?;

        let proto_members: Vec<_> = members.iter().map(admin_room_member_to_proto).collect();

        Ok(crate::proto::admin::GetRoomMembersResponse {
            members: proto_members,
        })
    }

    // === User Management ===

    pub async fn list_users(
        &self,
        req: crate::proto::admin::ListUsersRequest,
    ) -> Result<crate::proto::admin::ListUsersResponse, String> {
        let page = if req.page > 0 { req.page } else { 1 };
        let page_size = if req.page_size > 0 { req.page_size } else { 50 };

        let query = synctv_core::models::UserListQuery {
            page,
            page_size,
            ..Default::default()
        };

        let (users, total) = self.user_service.list_users(&query).await
            .map_err(|e| e.to_string())?;

        let user_list: Vec<_> = users.into_iter().map(|u| admin_user_to_proto(&u)).collect();

        Ok(crate::proto::admin::ListUsersResponse {
            users: user_list,
            total: total as i32,
        })
    }

    pub async fn get_user(
        &self,
        req: crate::proto::admin::GetUserRequest,
    ) -> Result<crate::proto::admin::GetUserResponse, String> {
        let uid = UserId::from_string(req.user_id);
        let user = self.user_service.get_user(&uid).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::GetUserResponse {
            user: Some(admin_user_to_proto(&user)),
        })
    }

    pub async fn update_user_role(
        &self,
        req: crate::proto::admin::UpdateUserRoleRequest,
    ) -> Result<crate::proto::admin::UpdateUserRoleResponse, String> {
        let uid = UserId::from_string(req.user_id);
        let user = self.user_service.get_user(&uid).await
            .map_err(|e| e.to_string())?;

        // Parse role from string
        let new_role = synctv_core::models::UserRole::from_str(&req.role)?;

        let updated_user = synctv_core::models::User {
            role: new_role,
            ..user
        };

        self.user_service.update_user(&updated_user).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::UpdateUserRoleResponse {
            user: Some(admin_user_to_proto(&updated_user)),
        })
    }

    pub async fn update_user_password(
        &self,
        req: crate::proto::admin::UpdateUserPasswordRequest,
    ) -> Result<crate::proto::admin::UpdateUserPasswordResponse, String> {
        let uid = UserId::from_string(req.user_id);

        self.user_service.set_password(&uid, &req.new_password).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::UpdateUserPasswordResponse {
            success: true,
        })
    }

    // === Settings Management ===

    pub async fn get_settings(
        &self,
        _req: crate::proto::admin::GetSettingsRequest,
    ) -> Result<crate::proto::admin::GetSettingsResponse, String> {
        let groups = self.settings_service.get_all().await
            .map_err(|e| e.to_string())?;

        let group_list: Vec<_> = groups.into_iter().map(|g| {
            crate::proto::admin::SettingsGroup {
                name: g.group.clone(),
                settings: g.value.into_bytes(),
            }
        }).collect();

        Ok(crate::proto::admin::GetSettingsResponse {
            groups: group_list,
        })
    }

    pub async fn get_settings_group(
        &self,
        req: crate::proto::admin::GetSettingsGroupRequest,
    ) -> Result<crate::proto::admin::GetSettingsGroupResponse, String> {
        let group = self.settings_service.get(&req.group).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::GetSettingsGroupResponse {
            group: Some(crate::proto::admin::SettingsGroup {
                name: group.group.clone(),
                settings: group.value.into_bytes(),
            }),
        })
    }

    pub async fn update_settings(
        &self,
        req: crate::proto::admin::UpdateSettingsRequest,
    ) -> Result<crate::proto::admin::UpdateSettingsResponse, String> {
        // Update each setting in the group
        for (key, value) in &req.settings {
            self.settings_service.update(key, value.clone()).await
                .map_err(|e| e.to_string())?;
        }

        Ok(crate::proto::admin::UpdateSettingsResponse {})
    }

    // === Email Management ===

    pub async fn send_test_email(
        &self,
        req: crate::proto::admin::SendTestEmailRequest,
    ) -> Result<crate::proto::admin::SendTestEmailResponse, String> {
        // Send test email using EmailService
        match self.email_service.send_test_email(&req.to).await {
            Ok(()) => Ok(crate::proto::admin::SendTestEmailResponse {
                message: format!("Test email sent successfully to {}", req.to),
                success: true,
            }),
            Err(e) => Ok(crate::proto::admin::SendTestEmailResponse {
                message: format!("Failed to send test email: {e}"),
                success: false,
            }),
        }
    }

    // === Provider Instance Management ===

    pub async fn list_provider_instances(
        &self,
        _req: crate::proto::admin::ListProviderInstancesRequest,
    ) -> Result<crate::proto::admin::ListProviderInstancesResponse, String> {
        let instances = self.provider_instance_manager
            .get_all_instances()
            .await
            .map_err(|e| e.to_string())?;

        let proto_instances: Vec<_> = instances
            .into_iter()
            .map(provider_instance_to_proto)
            .collect();

        Ok(crate::proto::admin::ListProviderInstancesResponse {
            instances: proto_instances,
        })
    }

    pub async fn add_provider_instance(
        &self,
        req: crate::proto::admin::AddProviderInstanceRequest,
    ) -> Result<crate::proto::admin::AddProviderInstanceResponse, String> {
        // Parse config if provided
        let (jwt_secret, custom_ca) = if req.config.is_empty() {
            (None, None)
        } else {
            let config: serde_json::Value = serde_json::from_slice(&req.config)
                .map_err(|e| format!("Invalid config JSON: {e}"))?;
            (
                config.get("jwt_secret").and_then(|v| v.as_str()).map(String::from),
                config.get("custom_ca").and_then(|v| v.as_str()).map(String::from),
            )
        };

        let instance = synctv_core::models::ProviderInstance {
            name: req.name,
            endpoint: req.endpoint,
            comment: if req.comment.is_empty() { None } else { Some(req.comment) },
            jwt_secret,
            custom_ca,
            timeout: req.timeout,
            tls: req.tls,
            insecure_tls: req.insecure_tls,
            providers: req.providers,
            enabled: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        self.provider_instance_manager
            .add(instance.clone())
            .await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::AddProviderInstanceResponse {
            instance: Some(provider_instance_to_proto(instance)),
        })
    }

    pub async fn update_provider_instance(
        &self,
        req: crate::proto::admin::UpdateProviderInstanceRequest,
    ) -> Result<crate::proto::admin::UpdateProviderInstanceResponse, String> {
        // Get existing instance
        let instances = self.provider_instance_manager.get_all_instances().await
            .map_err(|e| e.to_string())?;
        let mut instance = instances.into_iter()
            .find(|i| i.name == req.name)
            .ok_or_else(|| format!("Provider instance '{}' not found", req.name))?;

        // Update fields if provided
        if !req.endpoint.is_empty() {
            instance.endpoint = req.endpoint;
        }
        if !req.comment.is_empty() {
            instance.comment = Some(req.comment);
        }
        if !req.timeout.is_empty() {
            instance.timeout = req.timeout;
        }
        instance.tls = req.tls;
        instance.insecure_tls = req.insecure_tls;
        if !req.providers.is_empty() {
            instance.providers = req.providers;
        }

        // Parse config if provided
        if !req.config.is_empty() {
            let config: serde_json::Value = serde_json::from_slice(&req.config)
                .map_err(|e| format!("Invalid config JSON: {e}"))?;
            if let Some(jwt_secret) = config.get("jwt_secret").and_then(|v| v.as_str()) {
                instance.jwt_secret = Some(jwt_secret.to_string());
            }
            if let Some(custom_ca) = config.get("custom_ca").and_then(|v| v.as_str()) {
                instance.custom_ca = Some(custom_ca.to_string());
            }
        }

        instance.updated_at = chrono::Utc::now();

        self.provider_instance_manager
            .update(instance.clone())
            .await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::UpdateProviderInstanceResponse {
            instance: Some(provider_instance_to_proto(instance)),
        })
    }

    pub async fn delete_provider_instance(
        &self,
        req: crate::proto::admin::DeleteProviderInstanceRequest,
    ) -> Result<crate::proto::admin::DeleteProviderInstanceResponse, String> {
        self.provider_instance_manager
            .delete(&req.name)
            .await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::DeleteProviderInstanceResponse {
            success: true,
        })
    }

    pub async fn reconnect_provider_instance(
        &self,
        req: crate::proto::admin::ReconnectProviderInstanceRequest,
    ) -> Result<crate::proto::admin::ReconnectProviderInstanceResponse, String> {
        // Disable then enable to force reconnect
        self.provider_instance_manager.disable(&req.name).await
            .map_err(|e| e.to_string())?;
        self.provider_instance_manager.enable(&req.name).await
            .map_err(|e| e.to_string())?;

        // Get updated instance
        let instances = self.provider_instance_manager.get_all_instances().await
            .map_err(|e| e.to_string())?;
        let instance = instances.into_iter()
            .find(|i| i.name == req.name)
            .ok_or_else(|| format!("Provider instance '{}' not found", req.name))?;

        Ok(crate::proto::admin::ReconnectProviderInstanceResponse {
            instance: Some(provider_instance_to_proto(instance)),
        })
    }

    pub async fn enable_provider_instance(
        &self,
        req: crate::proto::admin::EnableProviderInstanceRequest,
    ) -> Result<crate::proto::admin::EnableProviderInstanceResponse, String> {
        self.provider_instance_manager.enable(&req.name).await
            .map_err(|e| e.to_string())?;

        // Get updated instance
        let instances = self.provider_instance_manager.get_all_instances().await
            .map_err(|e| e.to_string())?;
        let instance = instances.into_iter()
            .find(|i| i.name == req.name)
            .ok_or_else(|| format!("Provider instance '{}' not found", req.name))?;

        Ok(crate::proto::admin::EnableProviderInstanceResponse {
            instance: Some(provider_instance_to_proto(instance)),
        })
    }

    pub async fn disable_provider_instance(
        &self,
        req: crate::proto::admin::DisableProviderInstanceRequest,
    ) -> Result<crate::proto::admin::DisableProviderInstanceResponse, String> {
        self.provider_instance_manager.disable(&req.name).await
            .map_err(|e| e.to_string())?;

        // Get updated instance
        let instances = self.provider_instance_manager.get_all_instances().await
            .map_err(|e| e.to_string())?;
        let instance = instances.into_iter()
            .find(|i| i.name == req.name)
            .ok_or_else(|| format!("Provider instance '{}' not found", req.name))?;

        Ok(crate::proto::admin::DisableProviderInstanceResponse {
            instance: Some(provider_instance_to_proto(instance)),
        })
    }

    // === User Management (extended) ===

    pub async fn create_user(
        &self,
        req: crate::proto::admin::CreateUserRequest,
    ) -> Result<crate::proto::admin::CreateUserResponse, String> {
        if req.username.is_empty() || req.password.is_empty() || req.email.is_empty() {
            return Err("Username, password, and email are required".to_string());
        }

        let (user, _access, _refresh) = self.user_service
            .register(req.username.clone(), Some(req.email.clone()), req.password.clone())
            .await
            .map_err(|e| e.to_string())?;

        // Set role if specified
        let user = if !req.role.is_empty() && req.role != "user" {
            let new_role = UserRole::from_str(&req.role)?;
            let updated = synctv_core::models::User { role: new_role, ..user };
            self.user_service.update_user(&updated).await.map_err(|e| e.to_string())?;
            updated
        } else {
            user
        };

        Ok(crate::proto::admin::CreateUserResponse {
            user: Some(admin_user_to_proto(&user)),
        })
    }

    pub async fn delete_user(
        &self,
        req: crate::proto::admin::DeleteUserRequest,
    ) -> Result<crate::proto::admin::DeleteUserResponse, String> {
        let uid = UserId::from_string(req.user_id);
        let mut user = self.user_service.get_user(&uid).await.map_err(|e| e.to_string())?;

        if user.deleted_at.is_some() {
            return Err("User is already deleted".to_string());
        }

        user.deleted_at = Some(chrono::Utc::now());
        self.user_service.update_user(&user).await.map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::DeleteUserResponse { success: true })
    }

    pub async fn update_user_username(
        &self,
        req: crate::proto::admin::UpdateUserUsernameRequest,
    ) -> Result<crate::proto::admin::UpdateUserUsernameResponse, String> {
        let uid = UserId::from_string(req.user_id);

        if req.new_username.is_empty() {
            return Err("Username cannot be empty".to_string());
        }

        let mut user = self.user_service.get_user(&uid).await.map_err(|e| e.to_string())?;
        user.username = req.new_username;
        let updated = self.user_service.update_user(&user).await.map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::UpdateUserUsernameResponse {
            user: Some(admin_user_to_proto(&updated)),
        })
    }

    pub async fn ban_user(
        &self,
        req: crate::proto::admin::BanUserRequest,
    ) -> Result<crate::proto::admin::BanUserResponse, String> {
        let uid = UserId::from_string(req.user_id);
        let mut user = self.user_service.get_user(&uid).await.map_err(|e| e.to_string())?;

        if user.status == UserStatus::Banned {
            return Err("User is already banned".to_string());
        }

        user.status = UserStatus::Banned;
        let updated = self.user_service.update_user(&user).await.map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::BanUserResponse {
            user: Some(admin_user_to_proto(&updated)),
        })
    }

    pub async fn unban_user(
        &self,
        req: crate::proto::admin::UnbanUserRequest,
    ) -> Result<crate::proto::admin::UnbanUserResponse, String> {
        let uid = UserId::from_string(req.user_id);
        let mut user = self.user_service.get_user(&uid).await.map_err(|e| e.to_string())?;

        if user.status != UserStatus::Banned {
            return Err("User is not banned".to_string());
        }

        user.status = UserStatus::Active;
        let updated = self.user_service.update_user(&user).await.map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::UnbanUserResponse {
            user: Some(admin_user_to_proto(&updated)),
        })
    }

    pub async fn approve_user(
        &self,
        req: crate::proto::admin::ApproveUserRequest,
    ) -> Result<crate::proto::admin::ApproveUserResponse, String> {
        let uid = UserId::from_string(req.user_id);
        let mut user = self.user_service.get_user(&uid).await.map_err(|e| e.to_string())?;

        if user.status != UserStatus::Pending {
            return Err("User is not pending approval".to_string());
        }

        user.status = UserStatus::Active;
        let updated = self.user_service.update_user(&user).await.map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::ApproveUserResponse {
            user: Some(admin_user_to_proto(&updated)),
        })
    }

    pub async fn get_user_rooms(
        &self,
        req: crate::proto::admin::GetUserRoomsRequest,
    ) -> Result<crate::proto::admin::GetUserRoomsResponse, String> {
        let uid = UserId::from_string(req.user_id);

        // Get rooms created by user
        let (created_rooms, _) = self.room_service
            .list_rooms_by_creator(&uid, 1, 100)
            .await
            .map_err(|e| e.to_string())?;

        // Get rooms where user is a member
        let (joined_room_ids, _) = self.room_service
            .list_joined_rooms(&uid, 1, 100)
            .await
            .map_err(|e| e.to_string())?;

        let mut admin_rooms: Vec<crate::proto::admin::AdminRoom> = created_rooms
            .iter()
            .map(|r| admin_room_to_proto(r, None, self.connection_manager.room_connection_count(&r.id).try_into().ok()))
            .collect();

        // Add joined rooms not already in list
        for room_id in joined_room_ids {
            if admin_rooms.iter().any(|r| r.id == room_id.to_string()) {
                continue;
            }
            if let Ok(room) = self.room_service.get_room(&room_id).await {
                admin_rooms.push(admin_room_to_proto(
                    &room, None,
                    self.connection_manager.room_connection_count(&room.id).try_into().ok(),
                ));
            }
        }

        Ok(crate::proto::admin::GetUserRoomsResponse { rooms: admin_rooms })
    }

    // === Room Management (extended) ===

    pub async fn ban_room(
        &self,
        req: crate::proto::admin::BanRoomRequest,
    ) -> Result<crate::proto::admin::BanRoomResponse, String> {
        let rid = RoomId::from_string(req.room_id);
        let mut room = self.room_service.get_room(&rid).await.map_err(|e| e.to_string())?;

        if room.deleted_at.is_some() {
            return Err("Room is already banned".to_string());
        }

        room.deleted_at = Some(chrono::Utc::now());
        let updated = self.room_service.admin_update_room(&room).await.map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::BanRoomResponse {
            room: Some(admin_room_to_proto(
                &updated, None,
                self.connection_manager.room_connection_count(&rid).try_into().ok(),
            )),
        })
    }

    pub async fn unban_room(
        &self,
        req: crate::proto::admin::UnbanRoomRequest,
    ) -> Result<crate::proto::admin::UnbanRoomResponse, String> {
        let rid = RoomId::from_string(req.room_id);
        let mut room = self.room_service.get_room(&rid).await.map_err(|e| e.to_string())?;

        if room.deleted_at.is_none() {
            return Err("Room is not banned".to_string());
        }

        room.deleted_at = None;
        let updated = self.room_service.admin_update_room(&room).await.map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::UnbanRoomResponse {
            room: Some(admin_room_to_proto(
                &updated, None,
                self.connection_manager.room_connection_count(&rid).try_into().ok(),
            )),
        })
    }

    pub async fn approve_room(
        &self,
        req: crate::proto::admin::ApproveRoomRequest,
    ) -> Result<crate::proto::admin::ApproveRoomResponse, String> {
        let rid = RoomId::from_string(req.room_id);
        let room = self.room_service.approve_room(&rid).await.map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::ApproveRoomResponse {
            room: Some(admin_room_to_proto(
                &room, None,
                self.connection_manager.room_connection_count(&rid).try_into().ok(),
            )),
        })
    }

    pub async fn get_room_settings(
        &self,
        req: crate::proto::admin::GetRoomSettingsRequest,
    ) -> Result<crate::proto::admin::GetRoomSettingsResponse, String> {
        let rid = RoomId::from_string(req.room_id);
        let settings = self.room_service.get_room_settings(&rid).await.map_err(|e| e.to_string())?;
        let settings_json = serde_json::to_vec(&settings).map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::GetRoomSettingsResponse { settings: settings_json })
    }

    pub async fn update_room_settings(
        &self,
        req: crate::proto::admin::UpdateRoomSettingsRequest,
    ) -> Result<crate::proto::admin::UpdateRoomSettingsResponse, String> {
        let rid = RoomId::from_string(req.room_id);
        let settings: synctv_core::models::RoomSettings = serde_json::from_slice(&req.settings)
            .map_err(|e| format!("Invalid settings JSON: {e}"))?;

        self.room_service.set_room_settings(&rid, &settings).await.map_err(|e| e.to_string())?;

        let room = self.room_service.get_room(&rid).await.map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::UpdateRoomSettingsResponse {
            room: Some(admin_room_to_proto(
                &room, Some(&settings),
                self.connection_manager.room_connection_count(&rid).try_into().ok(),
            )),
        })
    }

    pub async fn reset_room_settings(
        &self,
        req: crate::proto::admin::ResetRoomSettingsRequest,
    ) -> Result<crate::proto::admin::ResetRoomSettingsResponse, String> {
        let rid = RoomId::from_string(req.room_id);
        self.room_service.reset_room_settings(&rid).await.map_err(|e| e.to_string())?;

        let room = self.room_service.get_room(&rid).await.map_err(|e| e.to_string())?;
        let settings = self.room_service.get_room_settings(&rid).await.unwrap_or_default();

        Ok(crate::proto::admin::ResetRoomSettingsResponse {
            room: Some(admin_room_to_proto(
                &room, Some(&settings),
                self.connection_manager.room_connection_count(&rid).try_into().ok(),
            )),
        })
    }

    // === Admin Management (root only) ===

    pub async fn add_admin(
        &self,
        req: crate::proto::admin::AddAdminRequest,
    ) -> Result<crate::proto::admin::AddAdminResponse, String> {
        let uid = UserId::from_string(req.user_id);
        let mut user = self.user_service.get_user(&uid).await.map_err(|e| e.to_string())?;

        if user.role.is_admin_or_above() {
            return Err("User is already an admin or root".to_string());
        }

        user.role = UserRole::Admin;
        let updated = self.user_service.update_user(&user).await.map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::AddAdminResponse {
            user: Some(admin_user_to_proto(&updated)),
        })
    }

    pub async fn remove_admin(
        &self,
        req: crate::proto::admin::RemoveAdminRequest,
    ) -> Result<crate::proto::admin::RemoveAdminResponse, String> {
        let uid = UserId::from_string(req.user_id);
        let mut user = self.user_service.get_user(&uid).await.map_err(|e| e.to_string())?;

        if matches!(user.role, UserRole::Root) {
            return Err("Cannot remove admin role from root user".to_string());
        }
        if !user.role.is_admin_or_above() {
            return Err("User is not an admin".to_string());
        }

        user.role = UserRole::User;
        self.user_service.update_user(&user).await.map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::RemoveAdminResponse { success: true })
    }

    pub async fn list_admins(
        &self,
        _req: crate::proto::admin::ListAdminsRequest,
    ) -> Result<crate::proto::admin::ListAdminsResponse, String> {
        let query = synctv_core::models::UserListQuery {
            page: 1,
            page_size: 1000,
            role: Some("admin".to_string()),
            ..Default::default()
        };

        let (users, _) = self.user_service.list_users(&query).await.map_err(|e| e.to_string())?;

        let admins: Vec<_> = users
            .into_iter()
            .filter(|u| u.role.is_admin_or_above())
            .map(|u| admin_user_to_proto(&u))
            .collect();

        Ok(crate::proto::admin::ListAdminsResponse { admins })
    }

    // === System Statistics ===

    pub async fn get_system_stats(
        &self,
        _req: crate::proto::admin::GetSystemStatsRequest,
    ) -> Result<crate::proto::admin::GetSystemStatsResponse, String> {
        let query_all = synctv_core::models::UserListQuery { page: 1, page_size: 1, ..Default::default() };
        let (_, total_users) = self.user_service.list_users(&query_all).await.unwrap_or((vec![], 0));

        let query_active = synctv_core::models::UserListQuery {
            page: 1, page_size: 1,
            status: Some("active".to_string()),
            ..Default::default()
        };
        let (_, active_users) = self.user_service.list_users(&query_active).await.unwrap_or((vec![], 0));

        let query_banned = synctv_core::models::UserListQuery {
            page: 1, page_size: 1,
            status: Some("banned".to_string()),
            ..Default::default()
        };
        let (_, banned_users) = self.user_service.list_users(&query_banned).await.unwrap_or((vec![], 0));

        let room_query_all = synctv_core::models::RoomListQuery { page: 1, page_size: 1, ..Default::default() };
        let (_, total_rooms) = self.room_service.list_rooms(&room_query_all).await.unwrap_or((vec![], 0));

        let room_query_active = synctv_core::models::RoomListQuery {
            page: 1, page_size: 1,
            status: Some(synctv_core::models::RoomStatus::Active),
            ..Default::default()
        };
        let (_, active_rooms) = self.room_service.list_rooms(&room_query_active).await.unwrap_or((vec![], 0));

        let room_query_banned = synctv_core::models::RoomListQuery {
            page: 1, page_size: 1,
            status: Some(synctv_core::models::RoomStatus::Banned),
            ..Default::default()
        };
        let (_, banned_rooms) = self.room_service.list_rooms(&room_query_banned).await.unwrap_or((vec![], 0));

        let provider_count = self.provider_instance_manager
            .get_all_instances().await
            .map(|i| i.len() as i32)
            .unwrap_or(0);

        Ok(crate::proto::admin::GetSystemStatsResponse {
            total_users: total_users as i32,
            active_users: active_users as i32,
            banned_users: banned_users as i32,
            total_rooms: total_rooms as i32,
            active_rooms: active_rooms as i32,
            banned_rooms: banned_rooms as i32,
            total_media: 0,
            provider_instances: provider_count,
            additional_stats: vec![],
        })
    }
}

// === Helper Functions ===

fn admin_room_to_proto(
    room: &synctv_core::models::Room,
    settings: Option<&synctv_core::models::RoomSettings>,
    member_count: Option<i32>,
) -> crate::proto::admin::AdminRoom {
    let room_settings = settings.cloned().unwrap_or_default();
    crate::proto::admin::AdminRoom {
        id: room.id.to_string(),
        name: room.name.clone(),
        description: room.description.clone(),
        creator_id: room.created_by.to_string(),
        creator_username: String::new(), // Would need to fetch user
        status: room.status.as_str().to_string(),
        settings: serde_json::to_vec(&room_settings).unwrap_or_default(),
        member_count: member_count.unwrap_or(0),
        created_at: room.created_at.timestamp(),
        updated_at: room.updated_at.timestamp(),
    }
}

fn admin_room_member_to_proto(member: &synctv_core::models::RoomMemberWithUser) -> crate::proto::admin::RoomMember {
    let role_str = match member.role {
        synctv_core::models::RoomRole::Creator => "creator",
        synctv_core::models::RoomRole::Admin => "admin",
        synctv_core::models::RoomRole::Member => "member",
        synctv_core::models::RoomRole::Guest => "guest",
    };

    crate::proto::admin::RoomMember {
        room_id: member.room_id.to_string(),
        user_id: member.user_id.to_string(),
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

fn admin_user_to_proto(user: &synctv_core::models::User) -> crate::proto::admin::AdminUser {
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

    crate::proto::admin::AdminUser {
        id: user.id.to_string(),
        username: user.username.clone(),
        email: user.email.clone().unwrap_or_default(),
        role: role_str.to_string(),
        status: status_str.to_string(),
        created_at: user.created_at.timestamp(),
        updated_at: user.updated_at.timestamp(),
    }
}

fn provider_instance_to_proto(instance: synctv_core::models::ProviderInstance) -> crate::proto::admin::ProviderInstance {
    // Generate status based on enabled flag
    let status = if instance.enabled {
        "connected".to_string()
    } else {
        "disabled".to_string()
    };

    crate::proto::admin::ProviderInstance {
        name: instance.name,
        endpoint: instance.endpoint,
        comment: instance.comment.unwrap_or_default(),
        timeout: instance.timeout,
        tls: instance.tls,
        insecure_tls: instance.insecure_tls,
        providers: instance.providers,
        enabled: instance.enabled,
        status,
        created_at: instance.created_at.timestamp(),
        updated_at: instance.updated_at.timestamp(),
    }
}
