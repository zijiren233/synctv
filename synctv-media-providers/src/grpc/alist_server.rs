//! Alist gRPC Server Implementation
//!
//! Thin wrapper around `AlistService` that implements gRPC server trait.

use super::alist::{
    alist_server::Alist, FsGetReq, FsGetResp, FsListReq, FsListResp, FsOtherReq, FsOtherResp,
    FsSearchReq, FsSearchResp, LoginReq, LoginResp, MeReq, MeResp,
};
use crate::alist::{AlistInterface, AlistService as AlistServiceImpl};
use tonic::{Request, Response, Status};

/// Validate that a host string is a non-empty, valid URL.
fn validate_host(host: &str) -> Result<(), Status> {
    if host.is_empty() {
        return Err(Status::invalid_argument("host must not be empty"));
    }
    url::Url::parse(host)
        .map_err(|e| Status::invalid_argument(format!("invalid host URL: {e}")))?;
    Ok(())
}

/// Alist gRPC server
///
/// Thin wrapper that delegates to `AlistService` for actual implementation.
pub struct AlistService {
    service: AlistServiceImpl,
}

impl AlistService {
    #[must_use] 
    pub const fn new() -> Self {
        Self {
            service: AlistServiceImpl::new(),
        }
    }
}

impl Default for AlistService {
    fn default() -> Self {
        Self::new()
    }
}

#[tonic::async_trait]
impl Alist for AlistService {
    async fn login(&self, request: Request<LoginReq>) -> Result<Response<LoginResp>, Status> {
        let req = request.into_inner();
        validate_host(&req.host)?;

        let token = self
            .service
            .login(req)
            .await
            .map_err(|e| Status::internal(format!("Login failed: {e}")))?;

        Ok(Response::new(LoginResp { token }))
    }

    async fn me(&self, request: Request<MeReq>) -> Result<Response<MeResp>, Status> {
        let req = request.into_inner();
        validate_host(&req.host)?;

        let resp = self
            .service
            .me(req)
            .await
            .map_err(|e| Status::internal(format!("me failed: {e}")))?;

        Ok(Response::new(resp))
    }

    async fn fs_get(&self, request: Request<FsGetReq>) -> Result<Response<FsGetResp>, Status> {
        let req = request.into_inner();
        validate_host(&req.host)?;

        let resp = self
            .service
            .fs_get(req)
            .await
            .map_err(|e| Status::internal(format!("fs_get failed: {e}")))?;

        Ok(Response::new(resp))
    }

    async fn fs_list(&self, request: Request<FsListReq>) -> Result<Response<FsListResp>, Status> {
        let req = request.into_inner();
        validate_host(&req.host)?;

        let resp = self
            .service
            .fs_list(req)
            .await
            .map_err(|e| Status::internal(format!("fs_list failed: {e}")))?;

        Ok(Response::new(resp))
    }

    async fn fs_other(
        &self,
        request: Request<FsOtherReq>,
    ) -> Result<Response<FsOtherResp>, Status> {
        let req = request.into_inner();
        validate_host(&req.host)?;

        let resp = self
            .service
            .fs_other(req)
            .await
            .map_err(|e| Status::internal(format!("fs_other failed: {e}")))?;

        Ok(Response::new(resp))
    }

    async fn fs_search(
        &self,
        request: Request<FsSearchReq>,
    ) -> Result<Response<FsSearchResp>, Status> {
        let req = request.into_inner();
        validate_host(&req.host)?;

        let resp = self
            .service
            .fs_search(req)
            .await
            .map_err(|e| Status::internal(format!("fs_search failed: {e}")))?;

        Ok(Response::new(resp))
    }
}
