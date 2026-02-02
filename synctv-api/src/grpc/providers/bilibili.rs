//! Bilibili Provider gRPC Service Implementation

use std::sync::Arc;
use tonic::{Request, Response, Status};

use crate::http::AppState;
use crate::impls::BilibiliApiImpl;

// Import generated proto types from synctv_proto
use crate::proto::providers::bilibili::bilibili_provider_service_server::{BilibiliProviderService, BilibiliProviderServiceServer};
use crate::proto::providers::bilibili::*;

/// Bilibili Provider gRPC Service
///
/// Thin wrapper that delegates to BilibiliApiImpl.
#[derive(Debug, Clone)]
pub struct BilibiliProviderGrpcService {
    app_state: Arc<AppState>,
}

impl BilibiliProviderGrpcService {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self { app_state }
    }
}

#[tonic::async_trait]
impl BilibiliProviderService for BilibiliProviderGrpcService {
    async fn parse(&self, request: Request<ParseRequest>) -> Result<Response<ParseResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Bilibili parse request: url={}", req.url);

        let instance_name = if req.instance_name.is_empty() {
            None
        } else {
            Some(req.instance_name.clone())
        };

        let api = BilibiliApiImpl::new(self.app_state.bilibili_provider.clone());

        api.parse(req, instance_name.as_deref())
            .await
            .map(Response::new)
            .map_err(|e| Status::internal(e))
    }

    async fn login_qr(&self, request: Request<LoginQrRequest>) -> Result<Response<QrCodeResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Bilibili login QR request");

        let instance_name = if req.instance_name.is_empty() {
            None
        } else {
            Some(req.instance_name.clone())
        };

        let api = BilibiliApiImpl::new(self.app_state.bilibili_provider.clone());

        api.login_qr(req, instance_name.as_deref())
            .await
            .map(Response::new)
            .map_err(|e| Status::internal(e))
    }

    async fn check_qr(&self, request: Request<CheckQrRequest>) -> Result<Response<QrStatusResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Bilibili check QR: {}", req.key);

        let instance_name = if req.instance_name.is_empty() {
            None
        } else {
            Some(req.instance_name.clone())
        };

        let api = BilibiliApiImpl::new(self.app_state.bilibili_provider.clone());

        api.check_qr(req, instance_name.as_deref())
            .await
            .map(Response::new)
            .map_err(|e| Status::internal(e))
    }

    async fn get_captcha(&self, request: Request<GetCaptchaRequest>) -> Result<Response<CaptchaResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Bilibili get captcha request");

        let instance_name = if req.instance_name.is_empty() {
            None
        } else {
            Some(req.instance_name.clone())
        };

        let api = BilibiliApiImpl::new(self.app_state.bilibili_provider.clone());

        api.get_captcha(req, instance_name.as_deref())
            .await
            .map(Response::new)
            .map_err(|e| Status::internal(e))
    }

    async fn send_sms(&self, request: Request<SendSmsRequest>) -> Result<Response<SendSmsResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Bilibili send SMS: phone={}", req.phone);

        let instance_name = if req.instance_name.is_empty() {
            None
        } else {
            Some(req.instance_name.clone())
        };

        let api = BilibiliApiImpl::new(self.app_state.bilibili_provider.clone());

        api.send_sms(req, instance_name.as_deref())
            .await
            .map(Response::new)
            .map_err(|e| Status::internal(e))
    }

    async fn login_sms(&self, request: Request<LoginSmsRequest>) -> Result<Response<LoginSmsResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Bilibili login SMS: phone={}", req.phone);

        let instance_name = if req.instance_name.is_empty() {
            None
        } else {
            Some(req.instance_name.clone())
        };

        let api = BilibiliApiImpl::new(self.app_state.bilibili_provider.clone());

        api.login_sms(req, instance_name.as_deref())
            .await
            .map(Response::new)
            .map_err(|e| Status::internal(e))
    }

    async fn get_user_info(&self, request: Request<UserInfoRequest>) -> Result<Response<UserInfoResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Bilibili user info request");

        let instance_name = if req.instance_name.is_empty() {
            None
        } else {
            Some(req.instance_name.clone())
        };

        let api = BilibiliApiImpl::new(self.app_state.bilibili_provider.clone());

        api.get_user_info(req, instance_name.as_deref())
            .await
            .map(Response::new)
            .map_err(|e| Status::internal(e))
    }

    async fn logout(&self, request: Request<LogoutRequest>) -> Result<Response<LogoutResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Bilibili logout request");

        let api = BilibiliApiImpl::new(self.app_state.bilibili_provider.clone());

        api.logout(req)
            .await
            .map(Response::new)
            .map_err(|e| Status::internal(e))
    }
}
