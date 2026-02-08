use std::sync::Arc;
use tonic::{Request, Response, Status};

use synctv_core::models::{ProviderInstance, RoomId, UserId, SettingsGroup as CoreSettingsGroup};
use synctv_core::service::{RemoteProviderManager, RoomService, UserService, SettingsService, SettingsRegistry};

// Use synctv_proto for all gRPC types to avoid duplication
use crate::proto::admin_service_server::AdminService;
use crate::proto::admin::{GetSettingsRequest, GetSettingsResponse, GetSettingsGroupRequest, GetSettingsGroupResponse, UpdateSettingsRequest, UpdateSettingsResponse, SendTestEmailRequest, SendTestEmailResponse, ListProviderInstancesRequest, ListProviderInstancesResponse, AddProviderInstanceRequest, AddProviderInstanceResponse, UpdateProviderInstanceRequest, UpdateProviderInstanceResponse, DeleteProviderInstanceRequest, DeleteProviderInstanceResponse, ReconnectProviderInstanceRequest, ReconnectProviderInstanceResponse, EnableProviderInstanceRequest, EnableProviderInstanceResponse, DisableProviderInstanceRequest, DisableProviderInstanceResponse, CreateUserRequest, CreateUserResponse, AdminUser, DeleteUserRequest, DeleteUserResponse, ListUsersRequest, ListUsersResponse, GetUserRequest, GetUserResponse, UpdateUserPasswordRequest, UpdateUserPasswordResponse, UpdateUserUsernameRequest, UpdateUserUsernameResponse, UpdateUserRoleRequest, UpdateUserRoleResponse, BanUserRequest, BanUserResponse, UnbanUserRequest, UnbanUserResponse, GetUserRoomsRequest, GetUserRoomsResponse, AdminRoom, ApproveUserRequest, ApproveUserResponse, ListRoomsRequest, ListRoomsResponse, GetRoomRequest, GetRoomResponse, UpdateRoomPasswordRequest, UpdateRoomPasswordResponse, DeleteRoomRequest, DeleteRoomResponse, BanRoomRequest, BanRoomResponse, UnbanRoomRequest, UnbanRoomResponse, ApproveRoomRequest, ApproveRoomResponse, GetRoomMembersRequest, GetRoomMembersResponse, AddAdminRequest, AddAdminResponse, RemoveAdminRequest, RemoveAdminResponse, ListAdminsRequest, ListAdminsResponse, GetSystemStatsRequest, GetSystemStatsResponse, GetRoomSettingsRequest, GetRoomSettingsResponse, UpdateRoomSettingsRequest, UpdateRoomSettingsResponse, ResetRoomSettingsRequest, ResetRoomSettingsResponse}; // Import all message types

/// `AdminService` implementation
#[derive(Clone)]
pub struct AdminServiceImpl {
    user_service: Arc<UserService>,
    room_service: Arc<RoomService>,
    provider_manager: Arc<RemoteProviderManager>,
    settings_service: Arc<SettingsService>,
    settings_registry: Option<Arc<SettingsRegistry>>,
    email_service: Option<Arc<synctv_core::service::EmailService>>,
}

impl AdminServiceImpl {
    #[must_use] 
    pub fn new(
        user_service: UserService,
        room_service: RoomService,
        provider_manager: Arc<RemoteProviderManager>,
        settings_service: Arc<SettingsService>,
        settings_registry: Option<Arc<SettingsRegistry>>,
        email_service: Option<Arc<synctv_core::service::EmailService>>,
    ) -> Self {
        Self {
            user_service: Arc::new(user_service),
            room_service: Arc::new(room_service),
            provider_manager,
            settings_service,
            settings_registry,
            email_service,
        }
    }

    /// Convert `ProviderInstance` to proto message
    fn instance_to_proto(
        &self,
        instance: &ProviderInstance,
    ) -> crate::proto::admin::ProviderInstance {
        // Determine status based on enabled flag
        // In production, this would do actual health checks via gRPC ping or health check endpoint
        let status = if instance.enabled {
            // Could check if gRPC connection is active in the provider manager
            "connected".to_string()
        } else {
            "disabled".to_string()
        };

        crate::proto::admin::ProviderInstance {
            name: instance.name.clone(),
            endpoint: instance.endpoint.clone(),
            comment: instance.comment.clone().unwrap_or_default(),
            timeout: instance.timeout.clone(),
            tls: instance.tls,
            insecure_tls: instance.insecure_tls,
            providers: instance.providers.clone(),
            enabled: instance.enabled,
            status,
            created_at: instance.created_at.timestamp(),
            updated_at: instance.updated_at.timestamp(),
        }
    }

    /// Convert `SettingsGroup` to proto message
    fn settings_group_to_proto(
        &self,
        group: &CoreSettingsGroup,
    ) -> crate::proto::admin::SettingsGroup {
        crate::proto::admin::SettingsGroup {
            name: group.key.clone(),
            settings: group.value.clone().into_bytes(),
        }
    }

    /// Check if user has admin role (load from database)
    async fn check_admin(&self, request: &Request<impl std::fmt::Debug>) -> Result<(), Status> {
        let user_context = request
            .extensions()
            .get::<super::interceptors::UserContext>()
            .ok_or_else(|| Status::unauthenticated("Authentication required"))?;

        let user_id = synctv_core::models::UserId::from_string(user_context.user_id.clone());

        // Load user from database to get current role
        let user = self
            .user_service
            .get_user(&user_id)
            .await
            .map_err(|_| Status::internal("Failed to get user"))?;

        // Check if user has admin role
        if !user.role.is_admin_or_above() {
            return Err(Status::permission_denied("Admin role required"));
        }
        Ok(())
    }

    /// Check if user has root role (load from database)
    async fn check_root(&self, request: &Request<impl std::fmt::Debug>) -> Result<(), Status> {
        let user_context = request
            .extensions()
            .get::<super::interceptors::UserContext>()
            .ok_or_else(|| Status::unauthenticated("Authentication required"))?;

        let user_id = synctv_core::models::UserId::from_string(user_context.user_id.clone());

        // Load user from database to get current role
        let user = self
            .user_service
            .get_user(&user_id)
            .await
            .map_err(|_| Status::internal("Failed to get user"))?;

        // Check if user has root role
        if !matches!(user.role, synctv_core::models::UserRole::Root) {
            return Err(Status::permission_denied("Root role required"));
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
        self.check_admin(&request).await?;

        let groups = self
            .settings_service
            .get_all()
            .await
            .map_err(|e| Status::internal(format!("Failed to get settings: {e}")))?;

        let proto_groups: Vec<_> = groups
            .into_iter()
            .map(|g| self.settings_group_to_proto(&g))
            .collect();

        tracing::info!("Retrieved {} settings groups", proto_groups.len());
        Ok(Response::new(GetSettingsResponse { groups: proto_groups }))
    }

    async fn get_settings_group(
        &self,
        request: Request<GetSettingsGroupRequest>,
    ) -> Result<Response<GetSettingsGroupResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();

        let group = self
            .settings_service
            .get(&req.group)
            .await
            .map_err(|e| Status::internal(format!("Failed to get settings group: {e}")))?;

        Ok(Response::new(GetSettingsGroupResponse {
            group: Some(self.settings_group_to_proto(&group)),
        }))
    }

    async fn update_settings(
        &self,
        request: Request<UpdateSettingsRequest>,
    ) -> Result<Response<UpdateSettingsResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();

        // Validate each setting value if registry is available
        if let Some(ref registry) = self.settings_registry {
            for (key, value) in &req.settings {
                // Construct full key path (group.key)
                let full_key = format!("{}.{}", req.group, key);

                // Validate the value using storage
                if !registry.storage.validate(&full_key, value) {
                    tracing::warn!("Invalid setting value: {} = {}", full_key, value);
                    return Err(Status::invalid_argument(format!(
                        "Invalid value '{value}' for setting '{full_key}'"
                    )));
                }
            }
        }

        // Update each setting individually
        for (key, value) in &req.settings {
            let full_key = format!("{}.{}", req.group, key);
            self.settings_service
                .update(&full_key, value.clone())
                .await
                .map_err(|e| Status::internal(format!("Failed to update setting '{full_key}': {e}")))?;
        }

        tracing::info!("Updated {} settings in group '{}'", req.settings.len(), req.group);
        Ok(Response::new(UpdateSettingsResponse {
            // Empty response
        }))
    }

    async fn send_test_email(
        &self,
        request: Request<SendTestEmailRequest>,
    ) -> Result<Response<SendTestEmailResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();

        let email_service = self
            .email_service
            .as_ref()
            .ok_or_else(|| Status::unavailable("Email service not configured"))?;

        match email_service.send_test_email(&req.to).await {
            Ok(()) => {
                tracing::info!("Test email sent successfully to '{}'", req.to);
                Ok(Response::new(SendTestEmailResponse {
                    success: true,
                    message: format!("Test email sent to {}", req.to),
                }))
            }
            Err(e) => {
                tracing::warn!("Failed to send test email to '{}': {}", req.to, e);
                Ok(Response::new(SendTestEmailResponse {
                    success: false,
                    message: format!("Failed to send test email: {e}"),
                }))
            }
        }
    }

    // =========================
    // Provider Instance Management
    // =========================

    async fn list_provider_instances(
        &self,
        request: Request<ListProviderInstancesRequest>,
    ) -> Result<Response<ListProviderInstancesResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();

        // Get all instances from database
        let instances = self
            .provider_manager
            .get_all_instances()
            .await
            .map_err(|e| Status::internal(format!("Failed to list provider instances: {e}")))?;

        // Filter by provider_type if specified
        let filtered_instances = if req.provider_type.is_empty() {
            instances
        } else {
            instances
                .into_iter()
                .filter(|inst| inst.providers.contains(&req.provider_type))
                .collect::<Vec<_>>()
        };

        // Convert to proto format
        let instances: Vec<crate::proto::admin::ProviderInstance> = filtered_instances
            .iter()
            .map(|inst| self.instance_to_proto(inst))
            .collect();

        tracing::info!("Listed {} provider instances", instances.len());

        Ok(Response::new(ListProviderInstancesResponse { instances }))
    }

    async fn add_provider_instance(
        &self,
        request: Request<AddProviderInstanceRequest>,
    ) -> Result<Response<AddProviderInstanceResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();

        // Validate input
        if req.name.trim().is_empty() {
            return Err(Status::invalid_argument(
                "Provider instance name cannot be empty",
            ));
        }
        if req.endpoint.trim().is_empty() {
            return Err(Status::invalid_argument("Endpoint cannot be empty"));
        }
        if req.providers.is_empty() {
            return Err(Status::invalid_argument(
                "At least one provider type must be specified",
            ));
        }

        // Parse additional config from JSON if provided
        let config: serde_json::Value = if req.config.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_slice(&req.config)
                .map_err(|e| Status::invalid_argument(format!("Invalid config JSON: {e}")))?
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
            jwt_secret: config
                .get("jwt_secret")
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string),
            custom_ca: config
                .get("custom_ca")
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string),
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

        // Add via RemoteProviderManager (creates gRPC connection + saves to DB)
        self.provider_manager
            .add(instance.clone())
            .await
            .map_err(|e| Status::internal(format!("Failed to add provider instance: {e}")))?;

        tracing::info!("Added provider instance: {}", req.name);

        Ok(Response::new(AddProviderInstanceResponse {
            instance: Some(self.instance_to_proto(&instance)),
        }))
    }

    async fn update_provider_instance(
        &self,
        request: Request<UpdateProviderInstanceRequest>,
    ) -> Result<Response<UpdateProviderInstanceResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();

        // Get existing instance
        let instances = self
            .provider_manager
            .get_all_instances()
            .await
            .map_err(|e| Status::internal(format!("Failed to get provider instances: {e}")))?;

        let existing = instances
            .iter()
            .find(|inst| inst.name == req.name)
            .ok_or_else(|| {
                Status::not_found(format!("Provider instance '{}' not found", req.name))
            })?;

        // Parse additional config from JSON if provided
        let config: Option<serde_json::Value> = if req.config.is_empty() {
            None
        } else {
            Some(
                serde_json::from_slice(&req.config)
                    .map_err(|e| Status::invalid_argument(format!("Invalid config JSON: {e}")))?,
            )
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
            jwt_secret: config
                .as_ref()
                .and_then(|c| c.get("jwt_secret"))
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string)
                .or_else(|| existing.jwt_secret.clone()),
            custom_ca: config
                .as_ref()
                .and_then(|c| c.get("custom_ca"))
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string)
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

        // Update via RemoteProviderManager (recreates gRPC connection + updates DB)
        self.provider_manager
            .update(updated_instance.clone())
            .await
            .map_err(|e| Status::internal(format!("Failed to update provider instance: {e}")))?;

        tracing::info!("Updated provider instance: {}", req.name);

        Ok(Response::new(UpdateProviderInstanceResponse {
            instance: Some(self.instance_to_proto(&updated_instance)),
        }))
    }

    async fn delete_provider_instance(
        &self,
        request: Request<DeleteProviderInstanceRequest>,
    ) -> Result<Response<DeleteProviderInstanceResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();

        // Delete via RemoteProviderManager (removes from DB + closes gRPC connection)
        self.provider_manager
            .delete(&req.name)
            .await
            .map_err(|e| Status::internal(format!("Failed to delete provider instance: {e}")))?;

        tracing::info!("Deleted provider instance: {}", req.name);

        Ok(Response::new(DeleteProviderInstanceResponse {
            success: true,
        }))
    }

    async fn reconnect_provider_instance(
        &self,
        request: Request<ReconnectProviderInstanceRequest>,
    ) -> Result<Response<ReconnectProviderInstanceResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();

        // Get existing instance
        let instances = self
            .provider_manager
            .get_all_instances()
            .await
            .map_err(|e| Status::internal(format!("Failed to get provider instances: {e}")))?;

        let existing = instances
            .iter()
            .find(|inst| inst.name == req.name)
            .ok_or_else(|| {
                Status::not_found(format!("Provider instance '{}' not found", req.name))
            })?
            .clone();

        // Update with same config (forces reconnection)
        self.provider_manager
            .update(existing.clone())
            .await
            .map_err(|e| {
                Status::internal(format!("Failed to reconnect provider instance: {e}"))
            })?;

        tracing::info!("Reconnected provider instance: {}", req.name);

        Ok(Response::new(ReconnectProviderInstanceResponse {
            instance: Some(self.instance_to_proto(&existing)),
        }))
    }

    async fn enable_provider_instance(
        &self,
        request: Request<EnableProviderInstanceRequest>,
    ) -> Result<Response<EnableProviderInstanceResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();

        // Enable via RemoteProviderManager (updates DB + creates gRPC connection)
        self.provider_manager
            .enable(&req.name)
            .await
            .map_err(|e| Status::internal(format!("Failed to enable provider instance: {e}")))?;

        // Get updated instance
        let instances = self
            .provider_manager
            .get_all_instances()
            .await
            .map_err(|e| Status::internal(format!("Failed to get provider instances: {e}")))?;

        let updated = instances
            .iter()
            .find(|inst| inst.name == req.name)
            .ok_or_else(|| {
                Status::not_found(format!("Provider instance '{}' not found", req.name))
            })?;

        tracing::info!("Enabled provider instance: {}", req.name);

        Ok(Response::new(EnableProviderInstanceResponse {
            instance: Some(self.instance_to_proto(updated)),
        }))
    }

    async fn disable_provider_instance(
        &self,
        request: Request<DisableProviderInstanceRequest>,
    ) -> Result<Response<DisableProviderInstanceResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();

        // Disable via RemoteProviderManager (updates DB + closes gRPC connection)
        self.provider_manager
            .disable(&req.name)
            .await
            .map_err(|e| Status::internal(format!("Failed to disable provider instance: {e}")))?;

        // Get updated instance
        let instances = self
            .provider_manager
            .get_all_instances()
            .await
            .map_err(|e| Status::internal(format!("Failed to get provider instances: {e}")))?;

        let updated = instances
            .iter()
            .find(|inst| inst.name == req.name)
            .ok_or_else(|| {
                Status::not_found(format!("Provider instance '{}' not found", req.name))
            })?;

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
        self.check_admin(&request).await?;
        let req = request.into_inner();

        // Validate input
        if req.username.is_empty() || req.password.is_empty() || req.email.is_empty() {
            return Err(Status::invalid_argument(
                "Username, password, and email are required",
            ));
        }

        // Register user via UserService
        let (user, _access_token, _refresh_token) = self
            .user_service
            .register(
                req.username.clone(),
                Some(req.email.clone()),
                req.password.clone(),
            )
            .await
            .map_err(|e| Status::internal(format!("Failed to create user: {e}")))?;

        // If role is specified, update role
        if !req.role.is_empty() && req.role != "user" {
            let mut updated_user = user.clone();
            updated_user.role = match req.role.as_str() {
                "admin" => synctv_core::models::UserRole::Admin,
                "root" => synctv_core::models::UserRole::Root,
                _ => user.role,
            };

            self.user_service
                .update_user(&updated_user)
                .await
                .map_err(|e| {
                    Status::internal(format!("Failed to update user role: {e}"))
                })?;
        }

        // Convert to AdminUser proto
        let admin_user = AdminUser {
            id: user.id.to_string(),
            username: user.username,
            email: user.email.unwrap_or_default(),
            role: if req.role.is_empty() {
                user.role.to_string()
            } else {
                req.role.clone()
            },
            status: user.status.as_str().to_string(),
            created_at: user.created_at.timestamp(),
            updated_at: user.updated_at.timestamp(),
        };

        Ok(Response::new(CreateUserResponse {
            user: Some(admin_user),
        }))
    }

    async fn delete_user(
        &self,
        request: Request<DeleteUserRequest>,
    ) -> Result<Response<DeleteUserResponse>, Status> {
        self.check_root(&request).await?; // Only root can delete users
        let req = request.into_inner();

        let user_id = UserId::from_string(req.user_id);

        // Get user to ensure they exist
        let user = self
            .user_service
            .get_user(&user_id)
            .await
            .map_err(|e| Status::not_found(format!("User not found: {e}")))?;

        // Check if user is already deleted
        if user.deleted_at.is_some() {
            return Err(Status::invalid_argument("User is already deleted"));
        }

        // Soft delete user by setting deleted_at
        let mut updated_user = user.clone();
        updated_user.deleted_at = Some(chrono::Utc::now());

        self.user_service
            .update_user(&updated_user)
            .await
            .map_err(|e| Status::internal(format!("Failed to delete user: {e}")))?;

        tracing::info!("User {} deleted by admin", user_id.as_str());

        Ok(Response::new(DeleteUserResponse { success: true }))
    }

    async fn list_users(
        &self,
        request: Request<ListUsersRequest>,
    ) -> Result<Response<ListUsersResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();

        // Build query
        let query = synctv_core::models::UserListQuery {
            page: if req.page == 0 { 1 } else { req.page },
            page_size: if req.page_size == 0 {
                20
            } else {
                req.page_size
            },
            search: if req.search.is_empty() {
                None
            } else {
                Some(req.search)
            },
            status: None,
            role: None,
        };

        // Get users
        let (users, total) = self
            .user_service
            .list_users(&query)
            .await
            .map_err(|e| Status::internal(format!("Failed to list users: {e}")))?;

        // Convert to AdminUser proto
        let admin_users = users
            .into_iter()
            .filter(|u| {
                // Filter by status if specified
                if !req.status.is_empty() {
                    let user_status = if u.deleted_at.is_some() {
                        "banned"
                    } else {
                        u.status.as_str()
                    };
                    if user_status != req.status {
                        return false;
                    }
                }

                // Filter by role if specified
                if !req.role.is_empty()
                    && u.role.as_str() != req.role {
                        return false;
                    }

                true
            })
            .map(|u| {
                let role = u.role.to_string();

                let status = if u.deleted_at.is_some() {
                    "banned".to_string()
                } else {
                    u.status.as_str().to_string()
                };

                AdminUser {
                    id: u.id.to_string(),
                    username: u.username,
                    email: u.email.unwrap_or_default(),
                    role,
                    status,
                    created_at: u.created_at.timestamp(),
                    updated_at: u.updated_at.timestamp(),
                }
            })
            .collect();

        Ok(Response::new(ListUsersResponse {
            users: admin_users,
            total: total as i32,
        }))
    }

    async fn get_user(
        &self,
        request: Request<GetUserRequest>,
    ) -> Result<Response<GetUserResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();

        let user_id = UserId::from_string(req.user_id);

        // Get user
        let user = self
            .user_service
            .get_user(&user_id)
            .await
            .map_err(|e| Status::not_found(format!("User not found: {e}")))?;

        // Convert to AdminUser proto
        let admin_user = AdminUser {
            id: user.id.to_string(),
            username: user.username,
            email: user.email.unwrap_or_default(),
            role: user.role.to_string(),
            status: if user.deleted_at.is_some() {
                "banned".to_string()
            } else {
                user.status.as_str().to_string()
            },
            created_at: user.created_at.timestamp(),
            updated_at: user.updated_at.timestamp(),
        };

        Ok(Response::new(GetUserResponse {
            user: Some(admin_user),
        }))
    }

    async fn update_user_password(
        &self,
        request: Request<UpdateUserPasswordRequest>,
    ) -> Result<Response<UpdateUserPasswordResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();

        let user_id = UserId::from_string(req.user_id);

        // Get user
        let mut user = self
            .user_service
            .get_user(&user_id)
            .await
            .map_err(|e| Status::not_found(format!("User not found: {e}")))?;

        // Hash new password
        let new_hash = synctv_core::service::auth::password::hash_password(&req.new_password)
            .await
            .map_err(|e| Status::internal(format!("Failed to hash password: {e}")))?;

        // Update password
        user.password_hash = new_hash;

        self.user_service
            .update_user(&user)
            .await
            .map_err(|e| Status::internal(format!("Failed to update password: {e}")))?;

        tracing::info!("Password updated for user {} by admin", user_id.as_str());

        Ok(Response::new(UpdateUserPasswordResponse { success: true }))
    }

    async fn update_user_username(
        &self,
        request: Request<UpdateUserUsernameRequest>,
    ) -> Result<Response<UpdateUserUsernameResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();

        let user_id = UserId::from_string(req.user_id);

        // Get user
        let mut user = self
            .user_service
            .get_user(&user_id)
            .await
            .map_err(|e| Status::not_found(format!("User not found: {e}")))?;

        // Validate new username
        if req.new_username.is_empty() {
            return Err(Status::invalid_argument("Username cannot be empty"));
        }

        // Update username
        user.username = req.new_username;

        let updated_user = self
            .user_service
            .update_user(&user)
            .await
            .map_err(|e| Status::internal(format!("Failed to update username: {e}")))?;

        tracing::info!("Username updated for user {} by admin", user_id.as_str());

        // Convert to AdminUser proto
        let admin_user = AdminUser {
            id: updated_user.id.to_string(),
            username: updated_user.username,
            email: updated_user.email.unwrap_or_default(),
            role: updated_user.role.to_string(),
            status: if updated_user.deleted_at.is_some() {
                "banned".to_string()
            } else {
                updated_user.status.as_str().to_string()
            },
            created_at: updated_user.created_at.timestamp(),
            updated_at: updated_user.updated_at.timestamp(),
        };

        Ok(Response::new(UpdateUserUsernameResponse {
            user: Some(admin_user),
        }))
    }

    async fn update_user_role(
        &self,
        request: Request<UpdateUserRoleRequest>,
    ) -> Result<Response<UpdateUserRoleResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();

        let user_id = UserId::from_string(req.user_id);

        // Get user
        let mut user = self
            .user_service
            .get_user(&user_id)
            .await
            .map_err(|e| Status::not_found(format!("User not found: {e}")))?;

        // Update role
        user.role = match req.role.as_str() {
            "user" => synctv_core::models::UserRole::User,
            "admin" => synctv_core::models::UserRole::Admin,
            "root" => synctv_core::models::UserRole::Root,
            _ => {
                return Err(Status::invalid_argument(
                    "Invalid role. Must be user, admin, or root",
                ))
            }
        };

        let updated_user = self
            .user_service
            .update_user(&user)
            .await
            .map_err(|e| Status::internal(format!("Failed to update role: {e}")))?;

        tracing::info!(
            "Role updated to {} for user {} by admin",
            req.role,
            user_id.as_str()
        );

        // Convert to AdminUser proto
        let admin_user = AdminUser {
            id: updated_user.id.to_string(),
            username: updated_user.username,
            email: updated_user.email.unwrap_or_default(),
            role: req.role.clone(),
            status: if updated_user.deleted_at.is_some() {
                "banned".to_string()
            } else {
                updated_user.status.as_str().to_string()
            },
            created_at: updated_user.created_at.timestamp(),
            updated_at: updated_user.updated_at.timestamp(),
        };

        Ok(Response::new(UpdateUserRoleResponse {
            user: Some(admin_user),
        }))
    }

    async fn ban_user(
        &self,
        request: Request<BanUserRequest>,
    ) -> Result<Response<BanUserResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();

        let user_id = UserId::from_string(req.user_id);

        // Get user
        let mut user = self
            .user_service
            .get_user(&user_id)
            .await
            .map_err(|e| Status::not_found(format!("User not found: {e}")))?;

        // Check if already banned
        if user.deleted_at.is_some() {
            return Err(Status::invalid_argument("User is already banned"));
        }

        // Ban user by setting deleted_at
        user.deleted_at = Some(chrono::Utc::now());

        let updated_user = self
            .user_service
            .update_user(&user)
            .await
            .map_err(|e| Status::internal(format!("Failed to ban user: {e}")))?;

        tracing::info!("User {} banned by admin", user_id.as_str());

        // Convert to AdminUser proto
        let admin_user = AdminUser {
            id: updated_user.id.to_string(),
            username: updated_user.username,
            email: updated_user.email.unwrap_or_default(),
            role: updated_user.role.to_string(),
            status: "banned".to_string(),
            created_at: updated_user.created_at.timestamp(),
            updated_at: updated_user.updated_at.timestamp(),
        };

        Ok(Response::new(BanUserResponse {
            user: Some(admin_user),
        }))
    }

    async fn unban_user(
        &self,
        request: Request<UnbanUserRequest>,
    ) -> Result<Response<UnbanUserResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();

        let user_id = UserId::from_string(req.user_id);

        // Get user
        let mut user = self
            .user_service
            .get_user(&user_id)
            .await
            .map_err(|e| Status::not_found(format!("User not found: {e}")))?;

        // Check if already active
        if user.deleted_at.is_none() {
            return Err(Status::invalid_argument("User is not banned"));
        }

        // Unban user by clearing deleted_at
        user.deleted_at = None;

        let updated_user = self
            .user_service
            .update_user(&user)
            .await
            .map_err(|e| Status::internal(format!("Failed to unban user: {e}")))?;

        tracing::info!("User {} unbanned by admin", user_id.as_str());

        // Convert to AdminUser proto
        let admin_user = AdminUser {
            id: updated_user.id.to_string(),
            username: updated_user.username,
            email: updated_user.email.unwrap_or_default(),
            role: updated_user.role.to_string(),
            status: "active".to_string(),
            created_at: updated_user.created_at.timestamp(),
            updated_at: updated_user.updated_at.timestamp(),
        };

        Ok(Response::new(UnbanUserResponse {
            user: Some(admin_user),
        }))
    }

    async fn get_user_rooms(
        &self,
        request: Request<GetUserRoomsRequest>,
    ) -> Result<Response<GetUserRoomsResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();

        let user_id = UserId::from_string(req.user_id);

        // Get rooms created by user
        let (created_rooms, _) = self
            .room_service
            .list_rooms_by_creator(&user_id, 1, 100) // Get first 100 created rooms
            .await
            .map_err(|e| Status::internal(format!("Failed to get created rooms: {e}")))?;

        // Get rooms where user is a member
        let (joined_room_ids, _) = self
            .room_service
            .list_joined_rooms(&user_id, 1, 100) // Get first 100 joined rooms
            .await
            .map_err(|e| Status::internal(format!("Failed to get joined rooms: {e}")))?;

        // Convert created rooms to AdminRoom proto
        let mut admin_rooms: Vec<AdminRoom> = Vec::new();
        for r in created_rooms {
            let member_count = 0; // Can be fetched if needed

            // Load room settings
            let settings = self.room_service
                .get_room_settings(&r.id)
                .await
                .unwrap_or_default();

            admin_rooms.push(AdminRoom {
                id: r.id.to_string(),
                name: r.name,
                description: r.description,
                creator_id: r.created_by.to_string(),
                creator_username: String::new(), // Can be fetched if needed
                status: match r.status {
                    synctv_core::models::RoomStatus::Pending => "pending".to_string(),

                    synctv_core::models::RoomStatus::Active => "active".to_string(),
                    synctv_core::models::RoomStatus::Banned => "banned".to_string(),
                },
                settings: serde_json::to_vec(&settings).unwrap_or_default(),
                member_count,
                created_at: r.created_at.timestamp(),
                updated_at: r.updated_at.timestamp(),
            });
        }

        // Add joined rooms
        for room_id in joined_room_ids {
            if let Ok(room) = self.room_service.get_room(&room_id).await {
                // Skip if already in list (creator)
                if admin_rooms.iter().any(|r| r.id == room.id.to_string()) {
                    continue;
                }

                let member_count = 0; // Can be fetched if needed

                // Load room settings
                let settings = self.room_service
                    .get_room_settings(&room.id)
                    .await
                    .unwrap_or_default();

                admin_rooms.push(AdminRoom {
                    id: room.id.to_string(),
                    name: room.name,
                    description: room.description,
                    creator_id: room.created_by.to_string(),
                    creator_username: String::new(), // Can be fetched if needed
                    status: match room.status {
                        synctv_core::models::RoomStatus::Pending => "pending".to_string(),

                        synctv_core::models::RoomStatus::Active => "active".to_string(),
                        synctv_core::models::RoomStatus::Banned => "banned".to_string(),
                    },
                    settings: serde_json::to_vec(&settings).unwrap_or_default(),
                    member_count,
                    created_at: room.created_at.timestamp(),
                    updated_at: room.updated_at.timestamp(),
                });
            }
        }

        Ok(Response::new(GetUserRoomsResponse { rooms: admin_rooms }))
    }

    async fn approve_user(
        &self,
        request: Request<ApproveUserRequest>,
    ) -> Result<Response<ApproveUserResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();

        let user_id = UserId::from_string(req.user_id);

        // Get user
        let user = self
            .user_service
            .get_user(&user_id)
            .await
            .map_err(|e| Status::not_found(format!("User not found: {e}")))?;

        // Note: The current implementation doesn't have a "pending" status
        // This method could be used to approve users pending verification
        // For now, it's a no-op that confirms the user exists and is active

        if user.deleted_at.is_some() {
            return Err(Status::invalid_argument("Cannot approve a banned user"));
        }

        tracing::info!("User {} approved by admin", user_id.as_str());

        // Convert to AdminUser proto
        let admin_user = AdminUser {
            id: user.id.to_string(),
            username: user.username,
            email: user.email.unwrap_or_default(),
            role: user.role.to_string(),
            status: user.status.as_str().to_string(),
            created_at: user.created_at.timestamp(),
            updated_at: user.updated_at.timestamp(),
        };

        Ok(Response::new(ApproveUserResponse {
            user: Some(admin_user),
        }))
    }

    // =========================
    // Room Management
    // =========================

    async fn list_rooms(
        &self,
        request: Request<ListRoomsRequest>,
    ) -> Result<Response<ListRoomsResponse>, Status> {
        self.check_admin(&request).await?;

        // Get user_id from request metadata (set by interceptor)
        let user_context = request
            .extensions()
            .get::<super::interceptors::UserContext>()
            .ok_or_else(|| Status::unauthenticated("Authentication required"))?;

        let user_id = synctv_core::models::UserId::from_string(user_context.user_id.clone());

        // Call service layer with gRPC types
        let req = request.into_inner();
        let response = self
            .room_service
            .list_rooms_grpc(req, &user_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to list rooms: {e}")))?;

        // Return gRPC response directly (no conversion needed)
        Ok(Response::new(response))
    }

    async fn get_room(
        &self,
        request: Request<GetRoomRequest>,
    ) -> Result<Response<GetRoomResponse>, Status> {
        self.check_admin(&request).await?;

        // Get user_id from request metadata (set by interceptor)
        let user_context = request
            .extensions()
            .get::<super::interceptors::UserContext>()
            .ok_or_else(|| Status::unauthenticated("Authentication required"))?;

        let user_id = synctv_core::models::UserId::from_string(user_context.user_id.clone());

        // Call service layer with gRPC types
        let req = request.into_inner();
        let response = self
            .room_service
            .get_room_grpc(req, &user_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get room: {e}")))?;

        // Return gRPC response directly (no conversion needed)
        Ok(Response::new(response))
    }

    async fn update_room_password(
        &self,
        request: Request<UpdateRoomPasswordRequest>,
    ) -> Result<Response<UpdateRoomPasswordResponse>, Status> {
        self.check_admin(&request).await?;

        // Get user_id from request metadata (set by interceptor)
        let user_context = request
            .extensions()
            .get::<super::interceptors::UserContext>()
            .ok_or_else(|| Status::unauthenticated("Authentication required"))?;

        let user_id = synctv_core::models::UserId::from_string(user_context.user_id.clone());

        // Call service layer with gRPC types
        let req = request.into_inner();
        let response = self
            .room_service
            .set_room_password(req, &user_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to set room password: {e}")))?;

        // Return gRPC response directly (no conversion needed)
        Ok(Response::new(response))
    }

    async fn delete_room(
        &self,
        request: Request<DeleteRoomRequest>,
    ) -> Result<Response<DeleteRoomResponse>, Status> {
        self.check_admin(&request).await?;

        // Get user_id from request metadata (set by interceptor)
        let user_context = request
            .extensions()
            .get::<super::interceptors::UserContext>()
            .ok_or_else(|| Status::unauthenticated("Authentication required"))?;

        let user_id = synctv_core::models::UserId::from_string(user_context.user_id.clone());

        // Call service layer with gRPC types
        let req = request.into_inner();
        let response = self
            .room_service
            .delete_room_grpc(req, &user_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to delete room: {e}")))?;

        // Return gRPC response directly (no conversion needed)
        Ok(Response::new(response))
    }

    async fn ban_room(
        &self,
        request: Request<BanRoomRequest>,
    ) -> Result<Response<BanRoomResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);

        // Get room
        let mut room = self
            .room_service
            .get_room(&room_id)
            .await
            .map_err(|e| Status::not_found(format!("Room not found: {e}")))?;

        // Check if already banned
        if room.deleted_at.is_some() {
            return Err(Status::invalid_argument("Room is already banned"));
        }

        // Ban room by setting deleted_at
        room.deleted_at = Some(chrono::Utc::now());

        let updated_room = self
            .room_service
            .admin_update_room(&room)
            .await
            .map_err(|e| Status::internal(format!("Failed to ban room: {e}")))?;

        tracing::info!("Admin banned room {}", room_id.as_str());

        // Get member count
        let member_count = self
            .room_service
            .get_member_count(&room_id)
            .await
            .unwrap_or(0);

        // Get creator username
        let creator_username = self
            .user_service
            .get_user(&updated_room.created_by)
            .await
            .map(|u| u.username)
            .unwrap_or_default();

        // Load room settings
        let settings = self.room_service
            .get_room_settings(&room_id)
            .await
            .unwrap_or_default();

        // Convert to AdminRoom proto
        let admin_room = AdminRoom {
            id: updated_room.id.to_string(),
            name: updated_room.name,
            description: updated_room.description,
            creator_id: updated_room.created_by.to_string(),
            creator_username,
            status: "banned".to_string(),
            settings: serde_json::to_vec(&settings).unwrap_or_default(),
            member_count,
            created_at: updated_room.created_at.timestamp(),
            updated_at: updated_room.updated_at.timestamp(),
        };

        Ok(Response::new(BanRoomResponse {
            room: Some(admin_room),
        }))
    }

    async fn unban_room(
        &self,
        request: Request<UnbanRoomRequest>,
    ) -> Result<Response<UnbanRoomResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);

        // Get room (need to query with deleted rooms)
        let room = self
            .room_service
            .get_room(&room_id)
            .await
            .map_err(|e| Status::not_found(format!("Room not found: {e}")))?;

        // Check if not banned
        if room.deleted_at.is_none() {
            return Err(Status::invalid_argument("Room is not banned"));
        }

        // Unban room by clearing deleted_at
        let mut room_to_unban = room;
        room_to_unban.deleted_at = None;

        let updated_room = self
            .room_service
            .admin_update_room(&room_to_unban)
            .await
            .map_err(|e| Status::internal(format!("Failed to unban room: {e}")))?;

        tracing::info!("Admin unbanned room {}", room_id.as_str());

        // Get member count
        let member_count = self
            .room_service
            .get_member_count(&room_id)
            .await
            .unwrap_or(0);

        // Get creator username
        let creator_username = self
            .user_service
            .get_user(&updated_room.created_by)
            .await
            .map(|u| u.username)
            .unwrap_or_default();

        // Load room settings
        let settings = self.room_service
            .get_room_settings(&room_id)
            .await
            .unwrap_or_default();

        // Convert to AdminRoom proto
        let admin_room = AdminRoom {
            id: updated_room.id.to_string(),
            name: updated_room.name,
            description: updated_room.description,
            creator_id: updated_room.created_by.to_string(),
            creator_username,
            status: match updated_room.status {
                synctv_core::models::RoomStatus::Pending => "pending".to_string(),

                synctv_core::models::RoomStatus::Active => "active".to_string(),
                synctv_core::models::RoomStatus::Banned => "banned".to_string(),
            },
            settings: serde_json::to_vec(&settings).unwrap_or_default(),
            member_count,
            created_at: updated_room.created_at.timestamp(),
            updated_at: updated_room.updated_at.timestamp(),
        };

        Ok(Response::new(UnbanRoomResponse {
            room: Some(admin_room),
        }))
    }

    async fn approve_room(
        &self,
        request: Request<ApproveRoomRequest>,
    ) -> Result<Response<ApproveRoomResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();
        let room_id = RoomId::from_string(req.room_id);

        // Approve the room (changes status from pending to active)
        let room = self
            .room_service
            .approve_room(&room_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to approve room {}: {}", room_id.as_str(), e);
                Status::internal(format!("Failed to approve room: {e}"))
            })?;

        tracing::info!("Admin approved room {}", room_id.as_str());

        // Get member count
        let member_count = self
            .room_service
            .get_member_count(&room_id)
            .await
            .unwrap_or(0);

        // Get creator username
        let creator_username = self
            .user_service
            .get_user(&room.created_by)
            .await
            .map(|u| u.username)
            .unwrap_or_default();

        // Load room settings
        let settings = self.room_service
            .get_room_settings(&room_id)
            .await
            .unwrap_or_default();

        // Convert to AdminRoom proto
        let admin_room = AdminRoom {
            id: room.id.to_string(),
            name: room.name,
            description: room.description,
            creator_id: room.created_by.to_string(),
            creator_username,
            status: match room.status {
                synctv_core::models::RoomStatus::Pending => "pending".to_string(),

                synctv_core::models::RoomStatus::Active => "active".to_string(),
                synctv_core::models::RoomStatus::Banned => "banned".to_string(),
            },
            settings: serde_json::to_vec(&settings).unwrap_or_default(),
            member_count,
            created_at: room.created_at.timestamp(),
            updated_at: room.updated_at.timestamp(),
        };

        Ok(Response::new(ApproveRoomResponse {
            room: Some(admin_room),
        }))
    }

    async fn get_room_members(
        &self,
        request: Request<GetRoomMembersRequest>,
    ) -> Result<Response<GetRoomMembersResponse>, Status> {
        // Check admin permission
        self.check_admin(&request).await?;

        // Get user_id from request metadata (set by interceptor)
        let user_context = request
            .extensions()
            .get::<super::interceptors::UserContext>()
            .ok_or_else(|| Status::unauthenticated("Authentication required"))?;

        let user_id = synctv_core::models::UserId::from_string(user_context.user_id.clone());

        // Call service layer with gRPC types
        let req = request.into_inner();
        let response = self
            .room_service
            .get_room_members_grpc(req, &user_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get room members: {e}")))?;

        // Return gRPC response directly (no conversion needed)
        Ok(Response::new(response))
    }

    // =========================
    // Admin Management (Root Only)
    // =========================

    async fn add_admin(
        &self,
        request: Request<AddAdminRequest>,
    ) -> Result<Response<AddAdminResponse>, Status> {
        self.check_root(&request).await?;
        let req = request.into_inner();

        let user_id = UserId::from_string(req.user_id);

        // Get user
        let mut user = self
            .user_service
            .get_user(&user_id)
            .await
            .map_err(|e| Status::not_found(format!("User not found: {e}")))?;

        // Check if already admin
        if user.role.is_admin_or_above() {
            return Err(Status::invalid_argument("User is already an admin or root"));
        }

        // Grant admin role
        user.role = synctv_core::models::UserRole::Admin;

        let updated_user = self
            .user_service
            .update_user(&user)
            .await
            .map_err(|e| Status::internal(format!("Failed to add admin: {e}")))?;

        tracing::info!("Root added admin role to user {}", user_id.as_str());

        // Convert to AdminUser proto
        let admin_user = AdminUser {
            id: updated_user.id.to_string(),
            username: updated_user.username,
            email: updated_user.email.unwrap_or_default(),
            role: "admin".to_string(),
            status: if updated_user.deleted_at.is_some() {
                "banned".to_string()
            } else {
                updated_user.status.as_str().to_string()
            },
            created_at: updated_user.created_at.timestamp(),
            updated_at: updated_user.updated_at.timestamp(),
        };

        Ok(Response::new(AddAdminResponse {
            user: Some(admin_user),
        }))
    }

    async fn remove_admin(
        &self,
        request: Request<RemoveAdminRequest>,
    ) -> Result<Response<RemoveAdminResponse>, Status> {
        self.check_root(&request).await?;
        let req = request.into_inner();

        let user_id = UserId::from_string(req.user_id);

        // Get user
        let mut user = self
            .user_service
            .get_user(&user_id)
            .await
            .map_err(|e| Status::not_found(format!("User not found: {e}")))?;

        // Check if admin
        if !user.role.is_admin_or_above() {
            return Err(Status::invalid_argument("User is not an admin"));
        }

        // Check if root (can't remove admin role from root)
        if matches!(user.role, synctv_core::models::UserRole::Root) {
            return Err(Status::invalid_argument(
                "Cannot remove admin role from root user",
            ));
        }

        // Remove admin role (set to user)
        user.role = synctv_core::models::UserRole::User;

        let _updated_user = self
            .user_service
            .update_user(&user)
            .await
            .map_err(|e| Status::internal(format!("Failed to remove admin: {e}")))?;

        tracing::info!("Root removed admin role from user {}", user_id.as_str());

        Ok(Response::new(RemoveAdminResponse { success: true }))
    }

    async fn list_admins(
        &self,
        request: Request<ListAdminsRequest>,
    ) -> Result<Response<ListAdminsResponse>, Status> {
        self.check_root(&request).await?;

        // Get all users (with filtering for active users)
        let query = synctv_core::models::UserListQuery {
            page: 1,
            page_size: 1000, // Get all admins (should be a small number)
            search: None,
            status: Some("active".to_string()),
            role: Some("admin".to_string()), // This will need to be implemented in UserService
        };

        let (users, _total) = self
            .user_service
            .list_users(&query)
            .await
            .map_err(|e| Status::internal(format!("Failed to list users: {e}")))?;

        // Filter for admin and root users
        let admin_users: Vec<AdminUser> = users
            .into_iter()
            .filter(|u| u.role.is_admin_or_above())
            .map(|u| {
                AdminUser {
                    id: u.id.to_string(),
                    username: u.username,
                    email: u.email.unwrap_or_default(),
                    role: u.role.to_string(),
                    status: if u.deleted_at.is_some() {
                        "banned".to_string()
                    } else {
                        u.status.as_str().to_string()
                    },
                    created_at: u.created_at.timestamp(),
                    updated_at: u.updated_at.timestamp(),
                }
            })
            .collect();

        Ok(Response::new(ListAdminsResponse {
            admins: admin_users,
        }))
    }

    // =========================
    // System Statistics
    // =========================

    async fn get_system_stats(
        &self,
        request: Request<GetSystemStatsRequest>,
    ) -> Result<Response<GetSystemStatsResponse>, Status> {
        self.check_admin(&request).await?;

        // Get user statistics using optimized queries
        // Note: This should ideally be done with a single query to the database
        // For now, we'll use multiple queries but they're all optimized with indexes

        // Get user counts (total, active, banned)
        let query_active = synctv_core::models::UserListQuery {
            page: 1,
            page_size: 1,
            search: None,
            status: Some("active".to_string()),
            role: None,
        };
        let query_banned = synctv_core::models::UserListQuery {
            page: 1,
            page_size: 1,
            search: None,
            status: Some("banned".to_string()),
            role: None,
        };
        let query_all = synctv_core::models::UserListQuery {
            page: 1,
            page_size: 1,
            search: None,
            status: None,
            role: None,
        };

        let (_, active_users) = self
            .user_service
            .list_users(&query_active)
            .await
            .unwrap_or((vec![], 0));
        let (_, banned_users) = self
            .user_service
            .list_users(&query_banned)
            .await
            .unwrap_or((vec![], 0));
        let (_, total_users) = self
            .user_service
            .list_users(&query_all)
            .await
            .unwrap_or((vec![], 0));

        // Get room counts
        let room_query_active = synctv_core::models::RoomListQuery {
            page: 1,
            page_size: 1,
            status: Some(synctv_core::models::RoomStatus::Active),
            search: None,
        };
        let room_query_closed = synctv_core::models::RoomListQuery {
            page: 1,
            page_size: 1,
            status: Some(synctv_core::models::RoomStatus::Banned),
            search: None,
        };
        let room_query_all = synctv_core::models::RoomListQuery {
            page: 1,
            page_size: 1,
            status: None,
            search: None,
        };

        let (_, active_rooms) = self
            .room_service
            .list_rooms(&room_query_active)
            .await
            .unwrap_or((vec![], 0));
        let (_, closed_rooms) = self
            .room_service
            .list_rooms(&room_query_closed)
            .await
            .unwrap_or((vec![], 0));
        let (_, total_rooms) = self
            .room_service
            .list_rooms(&room_query_all)
            .await
            .unwrap_or((vec![], 0));

        // Get provider instance count
        let provider_instances = self
            .provider_manager
            .get_all_instances()
            .await
            .map(|instances| instances.len() as i64)
            .unwrap_or(0);

        // Additional statistics can be added here
        let additional_stats = vec![];

        tracing::debug!(
            "System stats: users={}/{}, rooms={}/{}, providers={}",
            active_users,
            total_users,
            active_rooms,
            total_rooms,
            provider_instances
        );

        Ok(Response::new(GetSystemStatsResponse {
            total_users: total_users as i32,
            active_users: active_users as i32,
            banned_users: banned_users as i32,
            total_rooms: total_rooms as i32,
            active_rooms: active_rooms as i32,
            banned_rooms: closed_rooms as i32, // Using closed as "banned" for rooms
            total_media: 0,                    // Would require aggregating media across all rooms
            provider_instances: provider_instances as i32,
            additional_stats,
        }))
    }

    // =========================
    // Room Settings Management
    // =========================

    async fn get_room_settings(
        &self,
        request: Request<GetRoomSettingsRequest>,
    ) -> Result<Response<GetRoomSettingsResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();
        let room_id = synctv_core::models::RoomId::from_string(req.room_id);

        // Get room settings
        let settings = self
            .room_service
            .get_room_settings(&room_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get room settings: {e}")))?;

        // Serialize settings to JSON bytes
        let settings_json = serde_json::to_vec(&settings)
            .map_err(|e| Status::internal(format!("Failed to serialize settings: {e}")))?;

        Ok(Response::new(GetRoomSettingsResponse {
            settings: settings_json,
        }))
    }

    async fn update_room_settings(
        &self,
        request: Request<UpdateRoomSettingsRequest>,
    ) -> Result<Response<UpdateRoomSettingsResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();
        let room_id = synctv_core::models::RoomId::from_string(req.room_id);

        // Parse settings from JSON bytes
        let settings: synctv_core::models::RoomSettings = serde_json::from_slice(&req.settings)
            .map_err(|e| Status::invalid_argument(format!("Invalid settings JSON: {e}")))?;

        // Set room settings
        let updated_settings = self
            .room_service
            .set_room_settings(&room_id, &settings)
            .await
            .map_err(|e| Status::internal(format!("Failed to set room settings: {e}")))?;

        // Get the room to return in response
        let room = self
            .room_service
            .get_room(&room_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get room: {e}")))?;

        Ok(Response::new(UpdateRoomSettingsResponse {
            room: Some(crate::proto::admin::AdminRoom {
                id: room.id.as_str().to_string(),
                name: room.name.clone(),
                description: room.description.clone(),
                creator_id: room.created_by.as_str().to_string(),
                creator_username: String::new(),
                status: room.status.as_str().to_string(),
                settings: serde_json::to_vec(&updated_settings).unwrap_or_default(),
                member_count: 0,
                created_at: room.created_at.timestamp(),
                updated_at: room.updated_at.timestamp(),
            }),
        }))
    }

    async fn reset_room_settings(
        &self,
        request: Request<ResetRoomSettingsRequest>,
    ) -> Result<Response<ResetRoomSettingsResponse>, Status> {
        self.check_admin(&request).await?;
        let req = request.into_inner();
        let room_id = synctv_core::models::RoomId::from_string(req.room_id);

        // Reset room settings to default
        let _settings_json = self
            .room_service
            .reset_room_settings(&room_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to reset room settings: {e}")))?;

        // Get the room to return in response
        let room = self
            .room_service
            .get_room(&room_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get room: {e}")))?;

        // Get the updated settings
        let settings = self
            .room_service
            .get_room_settings(&room_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get room settings: {e}")))?;

        Ok(Response::new(ResetRoomSettingsResponse {
            room: Some(crate::proto::admin::AdminRoom {
                id: room.id.as_str().to_string(),
                name: room.name.clone(),
                description: room.description.clone(),
                creator_id: room.created_by.as_str().to_string(),
                creator_username: String::new(),
                status: room.status.as_str().to_string(),
                settings: serde_json::to_vec(&settings).unwrap_or_default(),
                member_count: 0,
                created_at: room.created_at.timestamp(),
                updated_at: room.updated_at.timestamp(),
            }),
        }))
    }
}
