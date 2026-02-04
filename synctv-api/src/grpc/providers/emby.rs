//! Emby Provider gRPC Service Implementation

use std::sync::Arc;
use tonic::{Request, Response, Status};

use crate::http::AppState;
use crate::impls::EmbyApiImpl;

// Import generated proto types from synctv_proto
use crate::proto::providers::emby::emby_provider_service_server::EmbyProviderService;
use crate::proto::providers::emby::{LoginRequest, LoginResponse, ListRequest, ListResponse, GetMeRequest, GetMeResponse, LogoutRequest, LogoutResponse, GetBindsRequest, GetBindsResponse, BindInfo};

/// Emby Provider gRPC Service
///
/// Thin wrapper that delegates to `EmbyApiImpl`.
#[derive(Clone)]
pub struct EmbyProviderGrpcService {
    app_state: Arc<AppState>,
}

impl EmbyProviderGrpcService {
    #[must_use] 
    pub const fn new(app_state: Arc<AppState>) -> Self {
        Self { app_state }
    }
}

#[tonic::async_trait]
impl EmbyProviderService for EmbyProviderGrpcService {
    async fn login(&self, request: Request<LoginRequest>) -> Result<Response<LoginResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Emby login request: host={}", req.host);

        let instance_name = if req.instance_name.is_empty() {
            None
        } else {
            Some(req.instance_name.clone())
        };

        let api = EmbyApiImpl::new(self.app_state.emby_provider.clone());

        api.login(req, instance_name.as_deref())
            .await
            .map(Response::new)
            .map_err(Status::internal)
    }

    async fn list(&self, request: Request<ListRequest>) -> Result<Response<ListResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Emby list request: host={}, path={}", req.host, req.path);

        let instance_name = if req.instance_name.is_empty() {
            None
        } else {
            Some(req.instance_name.clone())
        };

        let api = EmbyApiImpl::new(self.app_state.emby_provider.clone());

        api.list(req, instance_name.as_deref())
            .await
            .map(Response::new)
            .map_err(Status::internal)
    }

    async fn get_me(&self, request: Request<GetMeRequest>) -> Result<Response<GetMeResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Emby me request: host={}", req.host);

        let instance_name = if req.instance_name.is_empty() {
            None
        } else {
            Some(req.instance_name.clone())
        };

        let api = EmbyApiImpl::new(self.app_state.emby_provider.clone());

        api.get_me(req, instance_name.as_deref())
            .await
            .map(Response::new)
            .map_err(Status::internal)
    }

    async fn logout(&self, request: Request<LogoutRequest>) -> Result<Response<LogoutResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Emby logout request");

        let api = EmbyApiImpl::new(self.app_state.emby_provider.clone());

        api.logout(req)
            .await
            .map(Response::new)
            .map_err(Status::internal)
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
            .map_err(|e| Status::internal(format!("Failed to query credentials: {e}")))?;

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
