//! Emby gRPC Server Implementation
//!
//! Thin wrapper around `EmbyService` that implements gRPC server trait.

use super::emby::{
    emby_server::Emby, DeleteActiveEncodingsReq, Empty, FsListReq, FsListResp, GetItemReq,
    GetItemsReq, GetItemsResp, Item, LoginReq, LoginResp, LogoutReq, MeReq, MeResp,
    PlaybackInfoReq, PlaybackInfoResp, SystemInfoReq, SystemInfoResp,
};
use crate::emby::{EmbyInterface, EmbyService as EmbyServiceImpl};
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

/// Emby gRPC server
///
/// Thin wrapper that delegates to `EmbyService` for actual implementation.
pub struct EmbyService {
    service: EmbyServiceImpl,
}

impl EmbyService {
    #[must_use] 
    pub const fn new() -> Self {
        Self {
            service: EmbyServiceImpl::new(),
        }
    }
}

impl Default for EmbyService {
    fn default() -> Self {
        Self::new()
    }
}

#[tonic::async_trait]
impl Emby for EmbyService {
    async fn login(&self, request: Request<LoginReq>) -> Result<Response<LoginResp>, Status> {
        let req = request.into_inner();
        validate_host(&req.host)?;
        let resp = self.service.login(req).await
            .map_err(|e| Status::internal(format!("login failed: {e}")))?;
        Ok(Response::new(resp))
    }

    async fn me(&self, request: Request<MeReq>) -> Result<Response<MeResp>, Status> {
        let req = request.into_inner();
        validate_host(&req.host)?;
        let resp = self.service.me(req).await
            .map_err(|e| Status::internal(format!("me failed: {e}")))?;
        Ok(Response::new(resp))
    }

    async fn get_items(
        &self,
        request: Request<GetItemsReq>,
    ) -> Result<Response<GetItemsResp>, Status> {
        let req = request.into_inner();
        validate_host(&req.host)?;
        let resp = self.service.get_items(req).await
            .map_err(|e| Status::internal(format!("get_items failed: {e}")))?;
        Ok(Response::new(resp))
    }

    async fn get_item(&self, request: Request<GetItemReq>) -> Result<Response<Item>, Status> {
        let req = request.into_inner();
        validate_host(&req.host)?;
        let resp = self.service.get_item(req).await
            .map_err(|e| Status::internal(format!("get_item failed: {e}")))?;
        Ok(Response::new(resp))
    }

    async fn get_system_info(
        &self,
        request: Request<SystemInfoReq>,
    ) -> Result<Response<SystemInfoResp>, Status> {
        let req = request.into_inner();
        validate_host(&req.host)?;
        let resp = self.service.get_system_info(req).await
            .map_err(|e| Status::internal(format!("get_system_info failed: {e}")))?;
        Ok(Response::new(resp))
    }

    async fn fs_list(&self, request: Request<FsListReq>) -> Result<Response<FsListResp>, Status> {
        let req = request.into_inner();
        validate_host(&req.host)?;
        let resp = self.service.fs_list(req).await
            .map_err(|e| Status::internal(format!("fs_list failed: {e}")))?;
        Ok(Response::new(resp))
    }

    async fn logout(&self, request: Request<LogoutReq>) -> Result<Response<Empty>, Status> {
        let req = request.into_inner();
        validate_host(&req.host)?;
        let resp = self.service.logout(req).await
            .map_err(|e| Status::internal(format!("logout failed: {e}")))?;
        Ok(Response::new(resp))
    }

    async fn playback_info(
        &self,
        request: Request<PlaybackInfoReq>,
    ) -> Result<Response<PlaybackInfoResp>, Status> {
        let req = request.into_inner();
        validate_host(&req.host)?;
        let resp = self.service.playback_info(req).await
            .map_err(|e| Status::internal(format!("playback_info failed: {e}")))?;
        Ok(Response::new(resp))
    }

    async fn delete_active_encodings(
        &self,
        request: Request<DeleteActiveEncodingsReq>,
    ) -> Result<Response<Empty>, Status> {
        let req = request.into_inner();
        validate_host(&req.host)?;
        let resp = self.service.delete_active_encodings(req).await
            .map_err(|e| Status::internal(format!("delete_active_encodings failed: {e}")))?;
        Ok(Response::new(resp))
    }
}
