//! Alist Provider gRPC Service Implementation

use std::sync::Arc;
use tonic::{Request, Response, Status};
use synctv_core::provider::provider_client::{
    load_local_alist_client,
    create_remote_alist_client,
};

use crate::http::AppState;

// Generated proto code (included directly in this module)
mod proto {
    #![allow(clippy::all)]
    #![allow(warnings)]
    include!("proto/synctv.provider.alist.rs");
}

// Import generated proto types
use proto::{
    alist_provider_service_server::{AlistProviderService, AlistProviderServiceServer},
    *,
};

/// Alist Provider gRPC Service
///
/// This service wraps the internal Alist provider client and provides
/// a client-facing gRPC API with remote/local instance selection support.
#[derive(Debug, Clone)]
pub struct AlistProviderGrpcService {
    app_state: Arc<AppState>,
}

impl AlistProviderGrpcService {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self { app_state }
    }

    /// Get Alist client (remote or local) based on instance_name parameter
    fn get_client(&self, instance_name: &str) -> Arc<dyn synctv_providers::alist::AlistInterface> {
        if instance_name.is_empty() {
            return load_local_alist_client();
        }

        // Try to get remote instance
        let channel = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(
                self.app_state.provider_instance_manager.get(instance_name)
            )
        });

        if let Some(channel) = channel {
            tracing::debug!("Using remote Alist instance: {}", instance_name);
            create_remote_alist_client(channel)
        } else {
            tracing::warn!("Remote instance '{}' not found, falling back to local", instance_name);
            load_local_alist_client()
        }
    }
}

#[tonic::async_trait]
impl AlistProviderService for AlistProviderGrpcService {
    async fn login(&self, request: Request<LoginRequest>) -> Result<Response<LoginResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Alist login request: host={}, username={}", req.host, req.username);

        let client = self.get_client(&req.instance_name);

        // Determine password (use hashed if provided, otherwise plain)
        let (password, is_hashed) = if !req.hashed_password.is_empty() {
            (req.hashed_password, true)
        } else {
            (req.password, false)
        };

        let login_req = synctv_providers::grpc::alist::LoginReq {
            host: req.host,
            username: req.username,
            password,
            hashed: is_hashed,
        };

        let token = client.login(login_req).await
            .map_err(|e| Status::unauthenticated(format!("Login failed: {}", e)))?;

        Ok(Response::new(LoginResponse { token }))
    }

    async fn list(&self, request: Request<ListRequest>) -> Result<Response<ListResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Alist list request: host={}, path={}", req.host, req.path);

        let client = self.get_client(&req.instance_name);

        let list_req = synctv_providers::grpc::alist::FsListReq {
            host: req.host,
            token: req.token,
            path: req.path,
            password: req.password,
            page: req.page,
            per_page: req.per_page,
            refresh: req.refresh,
        };

        let resp = client.fs_list(list_req).await
            .map_err(|e| Status::internal(format!("List failed: {}", e)))?;

        // Convert FsListContent to FileItem
        let content: Vec<FileItem> = resp.content.into_iter().map(|item| FileItem {
            name: item.name,
            size: item.size,
            is_dir: item.is_dir,
            modified: item.modified,
            sign: item.sign,
            thumb: item.thumb,
            r#type: item.r#type,
        }).collect();

        Ok(Response::new(ListResponse {
            content,
            total: resp.total,
        }))
    }

    async fn get_me(&self, request: Request<GetMeRequest>) -> Result<Response<GetMeResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Alist me request: host={}", req.host);

        let client = self.get_client(&req.instance_name);

        let me_req = synctv_providers::grpc::alist::MeReq {
            host: req.host,
            token: req.token,
        };

        let resp = client.me(me_req).await
            .map_err(|e| Status::internal(format!("Get user info failed: {}", e)))?;

        Ok(Response::new(GetMeResponse {
            username: resp.username,
            base_path: resp.base_path,
        }))
    }

    async fn logout(&self, _request: Request<LogoutRequest>) -> Result<Response<LogoutResponse>, Status> {
        tracing::info!("gRPC Alist logout request");

        Ok(Response::new(LogoutResponse {
            message: "Logout successful".to_string(),
        }))
    }

    async fn get_binds(&self, request: Request<GetBindsRequest>) -> Result<Response<GetBindsResponse>, Status> {
        // Extract authenticated user from request extensions
        let auth_context = request.extensions().get::<crate::grpc::interceptors::UserContext>()
            .ok_or_else(|| Status::unauthenticated("Authentication required"))?;

        tracing::info!("gRPC Alist get binds request for user: {}", auth_context.user_id);

        // Query saved Alist credentials for current user
        let credentials = self.app_state.user_provider_credential_repository
            .get_by_user(&auth_context.user_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to query credentials: {}", e)))?;

        // Filter for Alist provider only and convert to BindInfo
        let binds: Vec<BindInfo> = credentials
            .into_iter()
            .filter(|c| c.provider == "alist")
            .map(|c| {
                // Parse credential data to extract host and username
                let host = c.credential_data.get("host")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                let username = c.credential_data.get("username")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                BindInfo {
                    id: c.id,
                    host,
                    username,
                    created_at: c.created_at.to_rfc3339(),
                }
            })
            .collect();

        Ok(Response::new(GetBindsResponse { binds }))
    }
}

/// Self-register Alist gRPC service on module load
pub fn init() {
    super::register_service_builder(|app_state, router| {
        tracing::info!("Registering Alist provider gRPC service");
        let service = AlistProviderGrpcService::new(app_state);
        router.add_service(AlistProviderServiceServer::new(service))
    });
}
