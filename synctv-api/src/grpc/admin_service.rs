use std::sync::Arc;
use tonic::{Request, Response, Status};

use synctv_core::service::{UserService, RoomService, ProviderInstanceManager};
use synctv_core::models::{RoomId, UserId, ProviderInstance};

use super::proto::admin::{
    admin_service_server::AdminService,
    *,
};

/// AdminService implementation
#[derive(Clone)]
pub struct AdminServiceImpl {
    user_service: Arc<UserService>,
    room_service: Arc<RoomService>,
    provider_manager: Arc<ProviderInstanceManager>,
}

impl AdminServiceImpl {
    pub fn new(
        user_service: UserService,
        room_service: RoomService,
        provider_manager: Arc<ProviderInstanceManager>,
    ) -> Self {
        Self {
            user_service: Arc::new(user_service),
            room_service: Arc::new(room_service),
            provider_manager,
        }
    }

    /// Convert ProviderInstance to proto message
    fn instance_to_proto(&self, instance: &ProviderInstance) -> super::proto::admin::ProviderInstance {
        super::proto::admin::ProviderInstance {
            name: instance.name.clone(),
            endpoint: instance.endpoint.clone(),
            comment: instance.comment.clone().unwrap_or_default(),
            timeout: instance.timeout.clone(),
            tls: instance.tls,
            insecure_tls: instance.insecure_tls,
            providers: instance.providers.clone(),
            enabled: instance.enabled,
            status: "connected".to_string(), // TODO: Implement actual health check
            created_at: instance.created_at.timestamp(),
            updated_at: instance.updated_at.timestamp(),
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
    // Provider Instance Management
    // =========================

    async fn list_provider_instances(
        &self,
        request: Request<ListProviderInstancesRequest>,
    ) -> Result<Response<ListProviderInstancesResponse>, Status> {
        self.check_admin(&request)?;
        let req = request.into_inner();

        // Get all instances from database
        let instances = self.provider_manager
            .get_all_instances()
            .await
            .map_err(|e| Status::internal(format!("Failed to list provider backends: {}", e)))?;

        // Filter by provider_type if specified
        let filtered_instances = if !req.provider_type.is_empty() {
            instances
                .into_iter()
                .filter(|inst| inst.providers.contains(&req.provider_type))
                .collect::<Vec<_>>()
        } else {
            instances
        };

        // Convert to proto format
        let instances: Vec<super::proto::admin::ProviderInstance> = filtered_instances
            .iter()
            .map(|inst| self.instance_to_proto(inst))
            .collect();

        tracing::info!("Listed {} provider instances", instances.len());

        Ok(Response::new(ListProviderInstancesResponse {
            instances,
        }))
    }

    async fn add_provider_instance(
        &self,
        request: Request<AddProviderInstanceRequest>,
    ) -> Result<Response<AddProviderInstanceResponse>, Status> {
        self.check_admin(&request)?;
        let req = request.into_inner();

        // Validate input
        if req.name.trim().is_empty() {
            return Err(Status::invalid_argument("Provider instance name cannot be empty"));
        }
        if req.endpoint.trim().is_empty() {
            return Err(Status::invalid_argument("Endpoint cannot be empty"));
        }
        if req.providers.is_empty() {
            return Err(Status::invalid_argument("At least one provider type must be specified"));
        }

        // Parse additional config from JSON if provided
        let config: serde_json::Value = if req.config.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_slice(&req.config)
                .map_err(|e| Status::invalid_argument(format!("Invalid config JSON: {}", e)))?
        };

        // Create ProviderInstance from request
        let instance = ProviderInstance {
            name: req.name.clone(),
            endpoint: req.endpoint.clone(),
            comment: if req.comment.is_empty() {
                None
            } else {
                Some(req.comment.clone())
            },
            jwt_secret: config.get("jwt_secret")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            custom_ca: config.get("custom_ca")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            timeout: if req.timeout.is_empty() {
                "10s".to_string()
            } else {
                req.timeout.clone()
            },
            tls: req.tls,
            insecure_tls: req.insecure_tls,
            providers: req.providers.clone(),
            enabled: true, // New instances are enabled by default
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        // Add via ProviderInstanceManager (creates gRPC connection + saves to DB)
        self.provider_manager
            .add(instance.clone())
            .await
            .map_err(|e| Status::internal(format!("Failed to add provider instance: {}", e)))?;

        tracing::info!("Added provider instance: {}", req.name);

        Ok(Response::new(AddProviderInstanceResponse {
            instance: Some(self.instance_to_proto(&instance)),
        }))
    }

    async fn update_provider_instance(
        &self,
        request: Request<UpdateProviderInstanceRequest>,
    ) -> Result<Response<UpdateProviderInstanceResponse>, Status> {
        self.check_admin(&request)?;
        let req = request.into_inner();

        // Get existing instance
        let instances = self.provider_manager.get_all_instances().await
            .map_err(|e| Status::internal(format!("Failed to get provider instances: {}", e)))?;

        let existing = instances
            .iter()
            .find(|inst| inst.name == req.name)
            .ok_or_else(|| Status::not_found(format!("Provider instance '{}' not found", req.name)))?;

        // Parse additional config from JSON if provided
        let config: Option<serde_json::Value> = if req.config.is_empty() {
            None
        } else {
            Some(serde_json::from_slice(&req.config)
                .map_err(|e| Status::invalid_argument(format!("Invalid config JSON: {}", e)))?)
        };

        // Build updated instance (use existing values if not provided)
        let updated_instance = ProviderInstance {
            name: existing.name.clone(), // Name (primary key) cannot be changed
            endpoint: if req.endpoint.is_empty() {
                existing.endpoint.clone()
            } else {
                req.endpoint.clone()
            },
            comment: if req.comment.is_empty() {
                existing.comment.clone()
            } else {
                Some(req.comment.clone())
            },
            jwt_secret: config.as_ref()
                .and_then(|c| c.get("jwt_secret"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| existing.jwt_secret.clone()),
            custom_ca: config.as_ref()
                .and_then(|c| c.get("custom_ca"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| existing.custom_ca.clone()),
            timeout: if req.timeout.is_empty() {
                existing.timeout.clone()
            } else {
                req.timeout.clone()
            },
            tls: req.tls,
            insecure_tls: req.insecure_tls,
            providers: if req.providers.is_empty() {
                existing.providers.clone()
            } else {
                req.providers.clone()
            },
            enabled: existing.enabled, // Don't change enabled status here (use enable/disable methods)
            created_at: existing.created_at,
            updated_at: chrono::Utc::now(),
        };

        // Update via ProviderInstanceManager (recreates gRPC connection + updates DB)
        self.provider_manager
            .update(updated_instance.clone())
            .await
            .map_err(|e| Status::internal(format!("Failed to update provider instance: {}", e)))?;

        tracing::info!("Updated provider instance: {}", req.name);

        Ok(Response::new(UpdateProviderInstanceResponse {
            instance: Some(self.instance_to_proto(&updated_instance)),
        }))
    }

    async fn delete_provider_instance(
        &self,
        request: Request<DeleteProviderInstanceRequest>,
    ) -> Result<Response<DeleteProviderInstanceResponse>, Status> {
        self.check_admin(&request)?;
        let req = request.into_inner();

        // Delete via ProviderInstanceManager (removes from DB + closes gRPC connection)
        self.provider_manager
            .delete(&req.name)
            .await
            .map_err(|e| Status::internal(format!("Failed to delete provider instance: {}", e)))?;

        tracing::info!("Deleted provider instance: {}", req.name);

        Ok(Response::new(DeleteProviderInstanceResponse {
            success: true,
        }))
    }

    async fn reconnect_provider_instance(
        &self,
        request: Request<ReconnectProviderInstanceRequest>,
    ) -> Result<Response<ReconnectProviderInstanceResponse>, Status> {
        self.check_admin(&request)?;
        let req = request.into_inner();

        // Get existing instance
        let instances = self.provider_manager.get_all_instances().await
            .map_err(|e| Status::internal(format!("Failed to get provider instances: {}", e)))?;

        let existing = instances
            .iter()
            .find(|inst| inst.name == req.name)
            .ok_or_else(|| Status::not_found(format!("Provider instance '{}' not found", req.name)))?
            .clone();

        // Update with same config (forces reconnection)
        self.provider_manager
            .update(existing.clone())
            .await
            .map_err(|e| Status::internal(format!("Failed to reconnect provider instance: {}", e)))?;

        tracing::info!("Reconnected provider instance: {}", req.name);

        Ok(Response::new(ReconnectProviderInstanceResponse {
            instance: Some(self.instance_to_proto(&existing)),
        }))
    }

    async fn enable_provider_instance(
        &self,
        request: Request<EnableProviderInstanceRequest>,
    ) -> Result<Response<EnableProviderInstanceResponse>, Status> {
        self.check_admin(&request)?;
        let req = request.into_inner();

        // Enable via ProviderInstanceManager (updates DB + creates gRPC connection)
        self.provider_manager
            .enable(&req.name)
            .await
            .map_err(|e| Status::internal(format!("Failed to enable provider instance: {}", e)))?;

        // Get updated instance
        let instances = self.provider_manager.get_all_instances().await
            .map_err(|e| Status::internal(format!("Failed to get provider instances: {}", e)))?;

        let updated = instances
            .iter()
            .find(|inst| inst.name == req.name)
            .ok_or_else(|| Status::not_found(format!("Provider instance '{}' not found", req.name)))?;

        tracing::info!("Enabled provider instance: {}", req.name);

        Ok(Response::new(EnableProviderInstanceResponse {
            instance: Some(self.instance_to_proto(updated)),
        }))
    }

    async fn disable_provider_instance(
        &self,
        request: Request<DisableProviderInstanceRequest>,
    ) -> Result<Response<DisableProviderInstanceResponse>, Status> {
        self.check_admin(&request)?;
        let req = request.into_inner();

        // Disable via ProviderInstanceManager (updates DB + closes gRPC connection)
        self.provider_manager
            .disable(&req.name)
            .await
            .map_err(|e| Status::internal(format!("Failed to disable provider instance: {}", e)))?;

        // Get updated instance
        let instances = self.provider_manager.get_all_instances().await
            .map_err(|e| Status::internal(format!("Failed to get provider instances: {}", e)))?;

        let updated = instances
            .iter()
            .find(|inst| inst.name == req.name)
            .ok_or_else(|| Status::not_found(format!("Provider instance '{}' not found", req.name)))?;

        tracing::info!("Disabled provider instance: {}", req.name);

        Ok(Response::new(DisableProviderInstanceResponse {
            instance: Some(self.instance_to_proto(updated)),
        }))
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
            provider_instances: 0,
            uptime_seconds: 0,
            additional_stats: vec![],
        }))
    }
}
