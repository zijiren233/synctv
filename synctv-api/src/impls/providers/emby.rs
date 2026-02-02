//! Emby API Implementation
//!
//! Unified implementation for all Emby API operations.
//! Used by both HTTP and gRPC handlers.

use std::sync::Arc;
use synctv_core::provider::EmbyProvider;
use crate::proto::providers::emby::*;

/// Emby API implementation
///
/// Contains all business logic for Emby operations.
/// Methods accept grpc-generated request types and return grpc-generated response types.
#[derive(Clone)]
pub struct EmbyApiImpl {
    provider: Arc<EmbyProvider>,
}

impl EmbyApiImpl {
    pub fn new(provider: Arc<EmbyProvider>) -> Self {
        Self { provider }
    }

    /// Login to Emby
    pub async fn login(&self, req: LoginRequest, instance_name: Option<&str>) -> Result<LoginResponse, String> {
        let user_info = self.provider
            .login(req.host, req.api_key, instance_name)
            .await
            .map_err(|e| e.to_string())?;

        // Extract admin status from user policy
        let is_admin = user_info.policy
            .as_ref()
            .map(|p| p.is_administrator)
            .unwrap_or(false);

        Ok(LoginResponse {
            user_id: user_info.id,
            username: user_info.name,
            is_admin,
        })
    }

    /// List Emby library items
    pub async fn list(&self, req: ListRequest, instance_name: Option<&str>) -> Result<ListResponse, String> {
        let list_req = synctv_providers::grpc::emby::FsListReq {
            host: req.host,
            token: req.token,
            path: req.path,
            start_index: req.start_index,
            limit: req.limit,
            search_term: req.search_term,
            user_id: req.user_id,
        };

        let resp = self.provider
            .fs_list(list_req, instance_name)
            .await
            .map_err(|e| e.to_string())?;

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

        Ok(ListResponse {
            items,
            total: resp.total,
        })
    }

    /// Get Emby user info
    pub async fn get_me(&self, req: GetMeRequest, instance_name: Option<&str>) -> Result<GetMeResponse, String> {
        let me_req = synctv_providers::grpc::emby::MeReq {
            host: req.host,
            token: req.token,
            user_id: String::new(), // Empty = get current user
        };

        let resp = self.provider
            .me(me_req, instance_name)
            .await
            .map_err(|e| e.to_string())?;

        Ok(GetMeResponse {
            id: resp.id,
            name: resp.name,
        })
    }

    /// Logout
    pub async fn logout(&self, _req: LogoutRequest) -> Result<LogoutResponse, String> {
        Ok(LogoutResponse {
            message: "Logout successful".to_string(),
        })
    }
}
