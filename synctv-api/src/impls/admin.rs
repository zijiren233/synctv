//! Admin API Implementation
//!
//! Unified implementation for all admin API operations.
//! Used by both HTTP and gRPC handlers.

use std::sync::Arc;
use std::str::FromStr;
use synctv_core::models::{UserId, RoomId};
use synctv_core::service::{RoomService, UserService, SettingsService, EmailService};

/// Admin API implementation
#[derive(Clone)]
pub struct AdminApiImpl {
    pub room_service: Arc<RoomService>,
    pub user_service: Arc<UserService>,
    pub settings_service: Arc<SettingsService>,
    pub email_service: Arc<EmailService>,
}

impl AdminApiImpl {
    pub fn new(
        room_service: Arc<RoomService>,
        user_service: Arc<UserService>,
        settings_service: Arc<SettingsService>,
        email_service: Arc<EmailService>,
    ) -> Self {
        Self {
            room_service,
            user_service,
            settings_service,
            email_service,
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

        let room_list: Vec<_> = rooms.into_iter().map(|r| admin_room_to_proto(&r)).collect();

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
            room: Some(admin_room_to_proto(&room)),
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

        self.room_service.update_room_password_grpc(req, &admin_user_id).await
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

        let proto_members: Vec<_> = members.into_iter().map(admin_room_member_to_proto).collect();

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

        // Update permissions based on role
        let role_permissions = match req.role.as_str() {
            "admin" | "creator" | "root" => synctv_core::models::PermissionBits(synctv_core::models::PermissionBits::ALL),
            _ => synctv_core::models::PermissionBits::default(),
        };

        let updated_user = synctv_core::models::User {
            permissions: role_permissions,
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

        self.user_service.update_password(&uid, &req.new_password).await
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
        let settings: synctv_core::models::SettingsGroup = serde_json::from_slice(&req.settings)
            .map_err(|e| e.to_string())?;

        self.settings_service.update(settings).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::UpdateSettingsResponse {})
    }

    // === Email Management ===

    pub async fn send_test_email(
        &self,
        req: crate::proto::admin::SendTestEmailRequest,
    ) -> Result<crate::proto::admin::SendTestEmailResponse, String> {
        let result = self.email_service.send_test_email(&req.to).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::admin::SendTestEmailResponse {
            message: result,
        })
    }

    // === Provider Instance Management ===

    pub async fn list_provider_instances(
        &self,
        _req: crate::proto::admin::ListProviderInstancesRequest,
    ) -> Result<crate::proto::admin::ListProviderInstancesResponse, String> {
        // This would require access to provider_instance_manager
        // For now, return empty list
        Ok(crate::proto::admin::ListProviderInstancesResponse {
            instances: vec![],
        })
    }

    pub async fn add_provider_instance(
        &self,
        _req: crate::proto::admin::AddProviderInstanceRequest,
    ) -> Result<crate::proto::admin::AddProviderInstanceResponse, String> {
        Err("Provider instance management not yet implemented".to_string())
    }

    pub async fn update_provider_instance(
        &self,
        _req: crate::proto::admin::UpdateProviderInstanceRequest,
    ) -> Result<crate::proto::admin::UpdateProviderInstanceResponse, String> {
        Err("Provider instance management not yet implemented".to_string())
    }

    pub async fn delete_provider_instance(
        &self,
        _req: crate::proto::admin::DeleteProviderInstanceRequest,
    ) -> Result<crate::proto::admin::DeleteProviderInstanceResponse, String> {
        Err("Provider instance management not yet implemented".to_string())
    }

    pub async fn reconnect_provider_instance(
        &self,
        _req: crate::proto::admin::ReconnectProviderInstanceRequest,
    ) -> Result<crate::proto::admin::ReconnectProviderInstanceResponse, String> {
        Err("Provider instance management not yet implemented".to_string())
    }

    pub async fn enable_provider_instance(
        &self,
        _req: crate::proto::admin::EnableProviderInstanceRequest,
    ) -> Result<crate::proto::admin::EnableProviderInstanceResponse, String> {
        Err("Provider instance management not yet implemented".to_string())
    }

    pub async fn disable_provider_instance(
        &self,
        _req: crate::proto::admin::DisableProviderInstanceRequest,
    ) -> Result<crate::proto::admin::DisableProviderInstanceResponse, String> {
        Err("Provider instance management not yet implemented".to_string())
    }
}

// === Helper Functions ===

fn admin_room_to_proto(room: &synctv_core::models::Room) -> crate::proto::admin::AdminRoom {
    crate::proto::admin::AdminRoom {
        id: room.id.to_string(),
        name: room.name.clone(),
        creator_id: room.created_by.to_string(),
        creator_username: String::new(), // Would need to fetch user
        status: room.status.as_str().to_string(),
        settings: serde_json::to_vec(&room.settings).unwrap_or_default(),
        member_count: room.member_count as i32,
        created_at: room.created_at.timestamp(),
        updated_at: room.updated_at.timestamp(),
    }
}

fn admin_room_member_to_proto(member: &synctv_core::models::RoomMemberWithUser) -> crate::proto::admin::RoomMember {
    crate::proto::admin::RoomMember {
        user_id: member.user_id.to_string(),
        username: member.username.clone(),
        role: member.role.to_string(),
        status: if member.is_online { "active" } else { "offline" },
        permissions: member.permissions.0,
    }
}

fn admin_user_to_proto(user: &synctv_core::models::User) -> crate::proto::admin::AdminUser {
    crate::proto::admin::AdminUser {
        id: user.id.to_string(),
        username: user.username.clone(),
        email: user.email.clone().unwrap_or_default(),
        role: "user".to_string(),  // Default - User model doesn't have role field
        permissions: user.permissions.0,
        status: if user.email_verified { "active" } else { "pending" },
        created_at: user.created_at.timestamp(),
        updated_at: user.updated_at.timestamp(),
    }
}
