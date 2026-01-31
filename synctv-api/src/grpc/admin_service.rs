use std::sync::Arc;
use tonic::{Request, Response, Status};

use synctv_core::service::{UserService, RoomService};
use synctv_core::models::{RoomId, UserId};

use super::proto::admin::{
    admin_service_server::AdminService,
    *,
};

/// AdminService implementation
#[derive(Clone)]
pub struct AdminServiceImpl {
    user_service: Arc<UserService>,
    room_service: Arc<RoomService>,
}

impl AdminServiceImpl {
    pub fn new(user_service: UserService, room_service: RoomService) -> Self {
        Self {
            user_service: Arc::new(user_service),
            room_service: Arc::new(room_service),
        }
    }

    /// Extract user_id from request extensions (set by auth interceptor)
    fn get_user_id(&self, request: &Request<impl std::fmt::Debug>) -> Result<UserId, Status> {
        let auth_context = request.extensions().get::<super::interceptors::AuthContext>()
            .ok_or_else(|| Status::unauthenticated("Authentication required"))?;
        Ok(UserId::from_string(auth_context.user_id.clone()))
    }

    /// Check if user has admin permissions
    fn check_admin(&self, request: &Request<impl std::fmt::Debug>) -> Result<(), Status> {
        let auth_context = request.extensions().get::<super::interceptors::AuthContext>()
            .ok_or_else(|| Status::unauthenticated("Authentication required"))?;

        // Check if user has admin permission (bit 63)
        const ADMIN_PERMISSION: i64 = 1 << 63;
        if (auth_context.permissions & ADMIN_PERMISSION) == 0 {
            return Err(Status::permission_denied("Admin permission required"));
        }
        Ok(())
    }

    /// Check if user has root permissions
    fn check_root(&self, request: &Request<impl std::fmt::Debug>) -> Result<(), Status> {
        let auth_context = request.extensions().get::<super::interceptors::AuthContext>()
            .ok_or_else(|| Status::unauthenticated("Authentication required"))?;

        // Check if user has root permission (bit 62)
        const ROOT_PERMISSION: i64 = 1 << 62;
        if (auth_context.permissions & ROOT_PERMISSION) == 0 {
            return Err(Status::permission_denied("Root permission required"));
        }
        Ok(())
    }
}

#[tonic::async_trait]
impl AdminService for AdminServiceImpl {
    // =========================
    // System Settings Management
    // =========================

    async fn get_settings(
        &self,
        request: Request<GetSettingsRequest>,
    ) -> Result<Response<GetSettingsResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement settings management
        Err(Status::unimplemented("GetSettings not yet implemented"))
    }

    async fn get_settings_group(
        &self,
        request: Request<GetSettingsGroupRequest>,
    ) -> Result<Response<GetSettingsGroupResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement settings group retrieval
        Err(Status::unimplemented("GetSettingsGroup not yet implemented"))
    }

    async fn update_settings(
        &self,
        request: Request<UpdateSettingsRequest>,
    ) -> Result<Response<UpdateSettingsResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement settings update
        Err(Status::unimplemented("UpdateSettings not yet implemented"))
    }

    async fn send_test_email(
        &self,
        request: Request<SendTestEmailRequest>,
    ) -> Result<Response<SendTestEmailResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement test email sending
        Err(Status::unimplemented("SendTestEmail not yet implemented"))
    }

    // =========================
    // Provider Backend Management
    // =========================

    async fn list_provider_backends(
        &self,
        request: Request<ListProviderBackendsRequest>,
    ) -> Result<Response<ListProviderBackendsResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement provider backend listing
        Ok(Response::new(ListProviderBackendsResponse {
            backends: vec![],
        }))
    }

    async fn add_provider_backend(
        &self,
        request: Request<AddProviderBackendRequest>,
    ) -> Result<Response<AddProviderBackendResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement add provider backend
        Err(Status::unimplemented("AddProviderBackend not yet implemented"))
    }

    async fn update_provider_backend(
        &self,
        request: Request<UpdateProviderBackendRequest>,
    ) -> Result<Response<UpdateProviderBackendResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement update provider backend
        Err(Status::unimplemented("UpdateProviderBackend not yet implemented"))
    }

    async fn delete_provider_backend(
        &self,
        request: Request<DeleteProviderBackendRequest>,
    ) -> Result<Response<DeleteProviderBackendResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement delete provider backend
        Err(Status::unimplemented("DeleteProviderBackend not yet implemented"))
    }

    async fn reconnect_provider_backend(
        &self,
        request: Request<ReconnectProviderBackendRequest>,
    ) -> Result<Response<ReconnectProviderBackendResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement reconnect provider backend
        Err(Status::unimplemented("ReconnectProviderBackend not yet implemented"))
    }

    async fn enable_provider_backend(
        &self,
        request: Request<EnableProviderBackendRequest>,
    ) -> Result<Response<EnableProviderBackendResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement enable provider backend
        Err(Status::unimplemented("EnableProviderBackend not yet implemented"))
    }

    async fn disable_provider_backend(
        &self,
        request: Request<DisableProviderBackendRequest>,
    ) -> Result<Response<DisableProviderBackendResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement disable provider backend
        Err(Status::unimplemented("DisableProviderBackend not yet implemented"))
    }

    // =========================
    // User Management
    // =========================

    async fn create_user(
        &self,
        request: Request<CreateUserRequest>,
    ) -> Result<Response<CreateUserResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement create user
        Err(Status::unimplemented("CreateUser not yet implemented"))
    }

    async fn delete_user(
        &self,
        request: Request<DeleteUserRequest>,
    ) -> Result<Response<DeleteUserResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement delete user
        Err(Status::unimplemented("DeleteUser not yet implemented"))
    }

    async fn list_users(
        &self,
        request: Request<ListUsersRequest>,
    ) -> Result<Response<ListUsersResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement list users with filters
        Ok(Response::new(ListUsersResponse {
            users: vec![],
            total: 0,
        }))
    }

    async fn get_user(
        &self,
        request: Request<GetUserRequest>,
    ) -> Result<Response<GetUserResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement get user
        Err(Status::unimplemented("GetUser not yet implemented"))
    }

    async fn update_user_password(
        &self,
        request: Request<UpdateUserPasswordRequest>,
    ) -> Result<Response<UpdateUserPasswordResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement update user password
        Err(Status::unimplemented("UpdateUserPassword not yet implemented"))
    }

    async fn update_user_username(
        &self,
        request: Request<UpdateUserUsernameRequest>,
    ) -> Result<Response<UpdateUserUsernameResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement update username
        Err(Status::unimplemented("UpdateUserUsername not yet implemented"))
    }

    async fn update_user_role(
        &self,
        request: Request<UpdateUserRoleRequest>,
    ) -> Result<Response<UpdateUserRoleResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement update user role
        Err(Status::unimplemented("UpdateUserRole not yet implemented"))
    }

    async fn ban_user(
        &self,
        request: Request<BanUserRequest>,
    ) -> Result<Response<BanUserResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement ban user
        Err(Status::unimplemented("BanUser not yet implemented"))
    }

    async fn unban_user(
        &self,
        request: Request<UnbanUserRequest>,
    ) -> Result<Response<UnbanUserResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement unban user
        Err(Status::unimplemented("UnbanUser not yet implemented"))
    }

    async fn get_user_rooms(
        &self,
        request: Request<GetUserRoomsRequest>,
    ) -> Result<Response<GetUserRoomsResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement get user rooms
        Ok(Response::new(GetUserRoomsResponse {
            rooms: vec![],
        }))
    }

    async fn approve_user(
        &self,
        request: Request<ApproveUserRequest>,
    ) -> Result<Response<ApproveUserResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement approve user
        Err(Status::unimplemented("ApproveUser not yet implemented"))
    }

    // =========================
    // Room Management
    // =========================

    async fn list_rooms(
        &self,
        request: Request<ListRoomsRequest>,
    ) -> Result<Response<ListRoomsResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement list rooms with filters
        Ok(Response::new(ListRoomsResponse {
            rooms: vec![],
            total: 0,
        }))
    }

    async fn get_room(
        &self,
        request: Request<GetRoomRequest>,
    ) -> Result<Response<GetRoomResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement get room
        Err(Status::unimplemented("GetRoom not yet implemented"))
    }

    async fn update_room_password(
        &self,
        request: Request<UpdateRoomPasswordRequest>,
    ) -> Result<Response<UpdateRoomPasswordResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement update room password
        Err(Status::unimplemented("UpdateRoomPassword not yet implemented"))
    }

    async fn delete_room(
        &self,
        request: Request<DeleteRoomRequest>,
    ) -> Result<Response<DeleteRoomResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement delete room
        Err(Status::unimplemented("DeleteRoom not yet implemented"))
    }

    async fn ban_room(
        &self,
        request: Request<BanRoomRequest>,
    ) -> Result<Response<BanRoomResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement ban room
        Err(Status::unimplemented("BanRoom not yet implemented"))
    }

    async fn unban_room(
        &self,
        request: Request<UnbanRoomRequest>,
    ) -> Result<Response<UnbanRoomResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement unban room
        Err(Status::unimplemented("UnbanRoom not yet implemented"))
    }

    async fn approve_room(
        &self,
        request: Request<ApproveRoomRequest>,
    ) -> Result<Response<ApproveRoomResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement approve room
        Err(Status::unimplemented("ApproveRoom not yet implemented"))
    }

    async fn get_room_members(
        &self,
        request: Request<GetRoomMembersRequest>,
    ) -> Result<Response<GetRoomMembersResponse>, Status> {
        self.check_admin(&request)?;
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);

        // Get room members
        let members = self.room_service
            .get_room_members(&room_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get room members: {}", e)))?;

        // Convert to response
        let member_list = members.into_iter().map(|m| {
            RoomMember {
                room_id: room_id.to_string(),
                user_id: m.user_id.to_string(),
                username: m.username,
                permissions: m.permissions.0,
                joined_at: m.joined_at.timestamp(),
                is_online: m.is_online,
            }
        }).collect();

        Ok(Response::new(GetRoomMembersResponse {
            members: member_list,
        }))
    }

    // =========================
    // Admin Management (Root Only)
    // =========================

    async fn add_admin(
        &self,
        request: Request<AddAdminRequest>,
    ) -> Result<Response<AddAdminResponse>, Status> {
        self.check_root(&request)?;
        // TODO: Implement add admin
        Err(Status::unimplemented("AddAdmin not yet implemented"))
    }

    async fn remove_admin(
        &self,
        request: Request<RemoveAdminRequest>,
    ) -> Result<Response<RemoveAdminResponse>, Status> {
        self.check_root(&request)?;
        // TODO: Implement remove admin
        Err(Status::unimplemented("RemoveAdmin not yet implemented"))
    }

    async fn list_admins(
        &self,
        request: Request<ListAdminsRequest>,
    ) -> Result<Response<ListAdminsResponse>, Status> {
        self.check_root(&request)?;
        // TODO: Implement list admins
        Ok(Response::new(ListAdminsResponse {
            admins: vec![],
        }))
    }

    // =========================
    // System Statistics
    // =========================

    async fn get_system_stats(
        &self,
        request: Request<GetSystemStatsRequest>,
    ) -> Result<Response<GetSystemStatsResponse>, Status> {
        self.check_admin(&request)?;
        // TODO: Implement system statistics
        Ok(Response::new(GetSystemStatsResponse {
            total_users: 0,
            active_users: 0,
            banned_users: 0,
            total_rooms: 0,
            active_rooms: 0,
            banned_rooms: 0,
            total_media: 0,
            provider_backends: 0,
            uptime_seconds: 0,
            additional_stats: vec![],
        }))
    }
}
