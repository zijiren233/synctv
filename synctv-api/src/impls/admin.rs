//! Admin API Implementation
//!
//! Unified implementation for all admin API operations.
//! Used by both HTTP and gRPC handlers.

use std::sync::Arc;
use std::str::FromStr;
use synctv_core::models::{UserId, RoomId};
use synctv_core::service::{RoomService, UserService, SettingsService, EmailService, ProviderInstanceManager};
use synctv_cluster::sync::ConnectionManager;

/// Admin API implementation
#[derive(Clone)]
pub struct AdminApiImpl {
    pub room_service: Arc<RoomService>,
    pub user_service: Arc<UserService>,
    pub settings_service: Arc<SettingsService>,
    pub email_service: Arc<EmailService>,
    pub connection_manager: Arc<ConnectionManager>,
    pub provider_instance_manager: Arc<ProviderInstanceManager>,
}

impl AdminApiImpl {
    #[must_use]
    pub const fn new(
        room_service: Arc<RoomService>,
        user_service: Arc<UserService>,
        settings_service: Arc<SettingsService>,
        email_service: Arc<EmailService>,
        connection_manager: Arc<ConnectionManager>,
        provider_instance_manager: Arc<ProviderInstanceManager>,
    ) -> Self {
        Self {
            room_service,
            user_service,
            settings_service,
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

    pub async fn set_room_password(
        &self,
        req: crate::proto::admin::SetRoomPasswordRequest,
    ) -> Result<crate::proto::admin::SetRoomPasswordResponse, String> {
        // For admin operations, use a system user ID (in real implementation, you'd get this from auth context)
        use synctv_core::models::UserId;
        let admin_user_id = UserId::new(); // System user

        self.room_service.set_room_password(req, &admin_user_id).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::SetRoomPasswordResponse {
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

    pub async fn set_user_role(
        &self,
        req: crate::proto::admin::SetUserRoleRequest,
    ) -> Result<crate::proto::admin::SetUserRoleResponse, String> {
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

        Ok(crate::proto::admin::SetUserRoleResponse {
            user: Some(admin_user_to_proto(&updated_user)),
        })
    }

    pub async fn set_user_password(
        &self,
        req: crate::proto::admin::SetUserPasswordRequest,
    ) -> Result<crate::proto::admin::SetUserPasswordResponse, String> {
        let uid = UserId::from_string(req.user_id);

        self.user_service.set_password(&uid, &req.new_password).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::SetUserPasswordResponse {
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

    pub async fn set_settings(
        &self,
        req: crate::proto::admin::SetSettingsRequest,
    ) -> Result<crate::proto::admin::SetSettingsResponse, String> {
        // Update each setting in the group
        for (key, value) in &req.settings {
            self.settings_service.update(key, value.clone()).await
                .map_err(|e| e.to_string())?;
        }

        Ok(crate::proto::admin::SetSettingsResponse {})
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

    pub async fn set_provider_instance(
        &self,
        req: crate::proto::admin::SetProviderInstanceRequest,
    ) -> Result<crate::proto::admin::SetProviderInstanceResponse, String> {
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

        Ok(crate::proto::admin::SetProviderInstanceResponse {
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
