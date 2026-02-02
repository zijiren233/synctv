//! Emby Provider gRPC Service Implementation

use std::sync::Arc;
use tonic::{Request, Response, Status};
use synctv_core::provider::provider_client::{
    load_local_emby_client,
    create_remote_emby_client,
};

use crate::http::AppState;

// Generated proto code (included directly in this module)
mod proto {
    #![allow(clippy::all)]
    #![allow(warnings)]
    include!("proto/synctv.provider.emby.rs");
}

// Import generated proto types
use proto::{
    emby_provider_service_server::{EmbyProviderService, EmbyProviderServiceServer},
    *,
};

/// Emby Provider gRPC Service
///
/// This service wraps the internal Emby provider client and provides
/// a client-facing gRPC API with remote/local instance selection support.
#[derive(Debug, Clone)]
pub struct EmbyProviderGrpcService {
    app_state: Arc<AppState>,
}

impl EmbyProviderGrpcService {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self { app_state }
    }

    /// Get Emby client (remote or local) based on instance_name parameter
    fn get_client(&self, instance_name: &str) -> Arc<dyn synctv_providers::emby::EmbyInterface> {
        if instance_name.is_empty() {
            return load_local_emby_client();
        }

        // Try to get remote instance
        let channel = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(
                self.app_state.provider_instance_manager.get(instance_name)
            )
        });

        if let Some(channel) = channel {
            tracing::debug!("Using remote Emby instance: {}", instance_name);
            create_remote_emby_client(channel)
        } else {
            tracing::warn!("Remote instance '{}' not found, falling back to local", instance_name);
            load_local_emby_client()
        }
    }
}

#[tonic::async_trait]
impl EmbyProviderService for EmbyProviderGrpcService {
    async fn login(&self, request: Request<LoginRequest>) -> Result<Response<LoginResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Emby login request: host={}", req.host);

        let client = self.get_client(&req.instance_name);

        // Validate API key by getting user info
        let me_req = synctv_providers::grpc::emby::MeReq {
            host: req.host,
            token: req.api_key,
            user_id: String::new(), // Empty = get current user
        };

        let user_info = client.me(me_req).await
            .map_err(|e| Status::unauthenticated(format!("Login failed: {}", e)))?;

        // Extract admin status from user policy
        let is_admin = user_info.policy
            .as_ref()
            .map(|p| p.is_administrator)
            .unwrap_or(false);

        Ok(Response::new(LoginResponse {
            user_id: user_info.id,
            username: user_info.name,
            is_admin,
        }))
    }

    async fn list(&self, request: Request<ListRequest>) -> Result<Response<ListResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Emby list request: host={}, path={}", req.host, req.path);

        let client = self.get_client(&req.instance_name);

        let list_req = synctv_providers::grpc::emby::FsListReq {
            host: req.host,
            token: req.token,
            path: req.path,
            start_index: req.start_index,
            limit: req.limit,
            search_term: req.search_term,
            user_id: req.user_id,
        };

        let resp = client.fs_list(list_req).await
            .map_err(|e| Status::internal(format!("List failed: {}", e)))?;

        // Convert Item to MediaItem
        let items: Vec<MediaItem> = resp.items.into_iter().map(|item| MediaItem {
            id: item.id,
            name: item.name,
            r#type: item.r#type,
            parent_id: item.parent_id,
            series_name: item.series_name,
            series_id: item.series_id,
            season_name: item.season_name,
        }).collect();

        Ok(Response::new(ListResponse {
            items,
            total: resp.total,
        }))
    }

    async fn get_me(&self, request: Request<GetMeRequest>) -> Result<Response<GetMeResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Emby me request: host={}", req.host);

        let client = self.get_client(&req.instance_name);

        let me_req = synctv_providers::grpc::emby::MeReq {
            host: req.host,
            token: req.token,
            user_id: String::new(), // Empty = get current user
        };

        let resp = client.me(me_req).await
            .map_err(|e| Status::internal(format!("Get user info failed: {}", e)))?;

        Ok(Response::new(GetMeResponse {
            id: resp.id,
            name: resp.name,
        }))
    }

    async fn logout(&self, _request: Request<LogoutRequest>) -> Result<Response<LogoutResponse>, Status> {
        tracing::info!("gRPC Emby logout request");

        Ok(Response::new(LogoutResponse {
            message: "Logout successful".to_string(),
        }))
    }

    async fn get_binds(&self, request: Request<GetBindsRequest>) -> Result<Response<GetBindsResponse>, Status> {
        // Extract authenticated user from request extensions
        let auth_context = request.extensions().get::<crate::grpc::interceptors::UserContext>()
            .ok_or_else(|| Status::unauthenticated("Authentication required"))?;

        tracing::info!("gRPC Emby get binds request for user: {}", auth_context.user_id);

        // Query saved Emby credentials for current user
        let credentials = self.app_state.user_provider_credential_repository
            .get_by_user(&auth_context.user_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to query credentials: {}", e)))?;

        // Filter for Emby provider only and convert to BindInfo
        let binds: Vec<BindInfo> = credentials
            .into_iter()
            .filter(|c| c.provider == "emby")
            .map(|c| {
                // Parse credential data to extract host and emby_user_id
                let host = c.credential_data.get("host")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                let emby_user_id = c.credential_data.get("emby_user_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                BindInfo {
                    id: c.id,
                    host,
                    user_id: emby_user_id,
                    created_at: c.created_at.to_rfc3339(),
                }
            })
            .collect();

        Ok(Response::new(GetBindsResponse { binds }))
    }
}

/// Self-register Emby gRPC service on module load

/// Register Emby gRPC service
///
/// # Arguments
/// - `app_state`: Application state
/// - `router`: Tonic router to add service to
///
/// # Returns
/// Router with Emby service added
pub fn register_service(
    app_state: Arc<crate::http::AppState>,
    router: tonic::transport::server::Router,
) -> tonic::transport::server::Router {
    let service = EmbyProviderGrpcService::new(app_state);
    router.add_service(EmbyProviderServiceServer::new(service))
}
