//! Alist Provider gRPC Service Implementation

use std::sync::Arc;
use tonic::{Request, Response, Status};

use crate::http::AppState;
use crate::impls::AlistApiImpl;

// Import generated proto types from synctv_proto
use crate::proto::providers::alist::alist_provider_service_server::AlistProviderService;
use crate::proto::providers::alist::*;

/// Alist Provider gRPC Service
///
/// Thin wrapper that delegates to AlistApiImpl.
#[derive(Clone)]
pub struct AlistProviderGrpcService {
    app_state: Arc<AppState>,
}

impl AlistProviderGrpcService {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self { app_state }
    }
}

#[tonic::async_trait]
impl AlistProviderService for AlistProviderGrpcService {
    async fn login(&self, request: Request<LoginRequest>) -> Result<Response<LoginResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Alist login request: host={}, username={}", req.host, req.username);

        let instance_name = if req.instance_name.is_empty() {
            None
        } else {
            Some(req.instance_name.clone())
        };

        let api = AlistApiImpl::new(self.app_state.alist_provider.clone());

        api.login(req, instance_name.as_deref())
            .await
            .map(Response::new)
            .map_err(Status::internal)
    }

    async fn list(&self, request: Request<ListRequest>) -> Result<Response<ListResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Alist list request: host={}, path={}", req.host, req.path);

        let instance_name = if req.instance_name.is_empty() {
            None
        } else {
            Some(req.instance_name.clone())
        };

        let api = AlistApiImpl::new(self.app_state.alist_provider.clone());

        api.list(req, instance_name.as_deref())
            .await
            .map(Response::new)
            .map_err(Status::internal)
    }

    async fn get_me(&self, request: Request<GetMeRequest>) -> Result<Response<GetMeResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Alist me request: host={}", req.host);

        let instance_name = if req.instance_name.is_empty() {
            None
        } else {
            Some(req.instance_name.clone())
        };

        let api = AlistApiImpl::new(self.app_state.alist_provider.clone());

        api.get_me(req, instance_name.as_deref())
            .await
            .map(Response::new)
            .map_err(Status::internal)
    }

    async fn logout(&self, request: Request<LogoutRequest>) -> Result<Response<LogoutResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Alist logout request");

        let api = AlistApiImpl::new(self.app_state.alist_provider.clone());

        api.logout(req)
            .await
            .map(Response::new)
            .map_err(Status::internal)
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
