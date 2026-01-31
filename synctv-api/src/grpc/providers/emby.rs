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
/// This service wraps the internal Emby vendor client and provides
/// a client-facing gRPC API with remote/local backend selection support.
#[derive(Debug, Clone)]
pub struct EmbyProviderGrpcService {
    app_state: Arc<AppState>,
}

impl EmbyProviderGrpcService {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self { app_state }
    }

    /// Get Emby client (remote or local) based on backend parameter
    fn get_client(&self, backend: &str) -> Arc<dyn synctv_providers::emby::EmbyInterface> {
        if backend.is_empty() {
            return load_local_emby_client();
        }

        // Try to get remote instance
        let channel = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(
                self.app_state.provider_instance_manager.get(backend)
            )
        });

        if let Some(channel) = channel {
            tracing::debug!("Using remote Emby instance: {}", backend);
            create_remote_emby_client(channel)
        } else {
            tracing::warn!("Remote instance '{}' not found, falling back to local", backend);
            load_local_emby_client()
        }
    }
}

#[tonic::async_trait]
impl EmbyProviderService for EmbyProviderGrpcService {
    async fn login(&self, request: Request<LoginRequest>) -> Result<Response<LoginResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Emby login request: host={}", req.host);

        let client = self.get_client(&req.backend);

        // Validate API key by getting user info
        let me_req = synctv_providers::grpc::emby::MeReq {
            host: req.host,
            token: req.api_key,
            user_id: String::new(), // Empty = get current user
        };

        let user_info = client.me(me_req).await
            .map_err(|e| Status::unauthenticated(format!("Login failed: {}", e)))?;

        Ok(Response::new(LoginResponse {
            user_id: user_info.id,
            username: user_info.name,
            is_admin: false, // TODO: Determine admin status from user policy
        }))
    }

    async fn list(&self, request: Request<ListRequest>) -> Result<Response<ListResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Emby list request: host={}, path={}", req.host, req.path);

        let client = self.get_client(&req.backend);

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

        let client = self.get_client(&req.backend);

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

    async fn get_binds(&self, _request: Request<GetBindsRequest>) -> Result<Response<GetBindsResponse>, Status> {
        tracing::info!("gRPC Emby get binds request");

        // TODO: Implement getting saved Emby credentials from database
        // This would query UserProviderCredential table

        Ok(Response::new(GetBindsResponse {
            binds: vec![],
        }))
    }
}

/// Self-register Emby gRPC service on module load
pub fn init() {
    super::register_service_builder(|app_state, router| {
        tracing::info!("Registering Emby provider gRPC service");
        let service = EmbyProviderGrpcService::new(app_state);
        router.add_service(EmbyProviderServiceServer::new(service))
    });
}
