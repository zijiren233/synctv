//! Bilibili Provider gRPC Service Implementation

use std::sync::Arc;
use tonic::{Request, Response, Status};
use synctv_core::provider::provider_client::{
    load_local_bilibili_client,
    create_remote_bilibili_client,
};

use crate::http::AppState;

// Generated proto code (included directly in this module)
mod proto {
    #![allow(clippy::all)]
    #![allow(warnings)]
    include!("proto/synctv.provider.bilibili.rs");
}

// Import generated proto types
use proto::{
    bilibili_provider_service_server::{BilibiliProviderService, BilibiliProviderServiceServer},
    *,
};

/// Bilibili Provider gRPC Service
///
/// This service wraps the internal Bilibili provider client and provides
/// a client-facing gRPC API with remote/local instance selection support.
#[derive(Debug, Clone)]
pub struct BilibiliProviderGrpcService {
    app_state: Arc<AppState>,
}

impl BilibiliProviderGrpcService {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self { app_state }
    }

    /// Get Bilibili client (remote or local) based on instance_name parameter
    fn get_client(&self, instance_name: &str) -> Arc<dyn synctv_providers::bilibili::BilibiliInterface> {
        if instance_name.is_empty() {
            return load_local_bilibili_client();
        }

        // Try to get remote instance
        let channel = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(
                self.app_state.provider_instance_manager.get(instance_name)
            )
        });

        if let Some(channel) = channel {
            tracing::debug!("Using remote Bilibili instance: {}", instance_name);
            create_remote_bilibili_client(channel)
        } else {
            tracing::warn!("Remote instance '{}' not found, falling back to local", instance_name);
            load_local_bilibili_client()
        }
    }
}

#[tonic::async_trait]
impl BilibiliProviderService for BilibiliProviderGrpcService {
    async fn parse(&self, request: Request<ParseRequest>) -> Result<Response<ParseResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Bilibili parse request: url={}", req.url);

        let client = self.get_client(&req.instance_name);

        // Step 1: Match URL
        let match_req = synctv_providers::grpc::bilibili::MatchReq {
            url: req.url.clone(),
        };

        let match_resp = client.r#match(match_req).await
            .map_err(|e| Status::invalid_argument(format!("Failed to match URL: {}", e)))?;

        // Step 2: Parse based on type
        let page_info = match match_resp.r#type.as_str() {
            "video" | "bv" | "av" => {
                let parse_req = synctv_providers::grpc::bilibili::ParseVideoPageReq {
                    cookies: req.cookies.clone(),
                    bvid: if match_resp.r#type == "bv" { match_resp.id.clone() } else { String::new() },
                    aid: if match_resp.r#type == "av" { match_resp.id.parse().unwrap_or(0) } else { 0 },
                    sections: false,
                };

                client.parse_video_page(parse_req).await
                    .map_err(|e| Status::internal(format!("Failed to parse video page: {}", e)))?
            }
            "pgc" | "ep" | "ss" => {
                let parse_req = synctv_providers::grpc::bilibili::ParsePgcPageReq {
                    cookies: req.cookies.clone(),
                    epid: if match_resp.r#type == "ep" { match_resp.id.parse().unwrap_or(0) } else { 0 },
                    ssid: if match_resp.r#type == "ss" { match_resp.id.parse().unwrap_or(0) } else { 0 },
                };

                client.parse_pgc_page(parse_req).await
                    .map_err(|e| Status::internal(format!("Failed to parse PGC page: {}", e)))?
            }
            "live" => {
                let parse_req = synctv_providers::grpc::bilibili::ParseLivePageReq {
                    cookies: req.cookies.clone(),
                    room_id: match_resp.id.parse().unwrap_or(0),
                };

                client.parse_live_page(parse_req).await
                    .map_err(|e| Status::internal(format!("Failed to parse live page: {}", e)))?
            }
            _ => {
                return Err(Status::invalid_argument(format!("Unsupported URL type: {}", match_resp.r#type)));
            }
        };

        // Convert to response format
        let videos: Vec<VideoInfo> = page_info.video_infos.into_iter().map(|v| VideoInfo {
            bvid: v.bvid,
            cid: v.cid as i64,
            epid: v.epid as i64,
            name: v.name,
            cover: v.cover_image,
            is_live: v.live,
        }).collect();

        // Parse actors string (comma-separated) into Vec
        let actors = if page_info.actors.is_empty() {
            vec![]
        } else {
            page_info.actors.split(',').map(|s| s.trim().to_string()).collect()
        };

        Ok(Response::new(ParseResponse {
            title: page_info.title,
            actors,
            videos,
        }))
    }

    async fn login_qr(&self, request: Request<LoginQrRequest>) -> Result<Response<QrCodeResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Bilibili login QR request");

        let client = self.get_client(&req.instance_name);

        let resp = client.new_qr_code(synctv_providers::grpc::bilibili::Empty {}).await
            .map_err(|e| Status::internal(format!("Failed to generate QR code: {}", e)))?;

        Ok(Response::new(QrCodeResponse {
            url: resp.url,
            key: resp.key,
        }))
    }

    async fn check_qr(&self, request: Request<CheckQrRequest>) -> Result<Response<QrStatusResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Bilibili check QR: {}", req.key);

        let client = self.get_client(&req.instance_name);

        let check_req = synctv_providers::grpc::bilibili::LoginWithQrCodeReq {
            key: req.key,
        };

        let resp = client.login_with_qr_code(check_req).await
            .map_err(|e| Status::internal(format!("Failed to check QR status: {}", e)))?;

        Ok(Response::new(QrStatusResponse {
            status: resp.status,
            cookies: resp.cookies,
        }))
    }

    async fn get_captcha(&self, request: Request<GetCaptchaRequest>) -> Result<Response<CaptchaResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Bilibili get captcha request");

        let client = self.get_client(&req.instance_name);

        let resp = client.new_captcha(synctv_providers::grpc::bilibili::Empty {}).await
            .map_err(|e| Status::internal(format!("Failed to get captcha: {}", e)))?;

        Ok(Response::new(CaptchaResponse {
            token: resp.token,
            gt: resp.gt,
            challenge: resp.challenge,
        }))
    }

    async fn send_sms(&self, request: Request<SendSmsRequest>) -> Result<Response<SendSmsResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Bilibili send SMS: phone={}", req.phone);

        let client = self.get_client(&req.instance_name);

        let sms_req = synctv_providers::grpc::bilibili::NewSmsReq {
            phone: req.phone,
            token: req.token,
            challenge: req.challenge,
            validate: req.validate,
        };

        let resp = client.new_sms(sms_req).await
            .map_err(|e| Status::internal(format!("Failed to send SMS: {}", e)))?;

        Ok(Response::new(SendSmsResponse {
            captcha_key: resp.captcha_key,
        }))
    }

    async fn login_sms(&self, request: Request<LoginSmsRequest>) -> Result<Response<LoginSmsResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Bilibili login SMS: phone={}", req.phone);

        let client = self.get_client(&req.instance_name);

        let login_req = synctv_providers::grpc::bilibili::LoginWithSmsReq {
            phone: req.phone,
            code: req.code,
            captcha_key: req.captcha_key,
        };

        let resp = client.login_with_sms(login_req).await
            .map_err(|e| Status::internal(format!("Failed to login with SMS: {}", e)))?;

        Ok(Response::new(LoginSmsResponse {
            cookies: resp.cookies,
        }))
    }

    async fn get_user_info(&self, request: Request<UserInfoRequest>) -> Result<Response<UserInfoResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("gRPC Bilibili user info request");

        let client = self.get_client(&req.instance_name);

        let info_req = synctv_providers::grpc::bilibili::UserInfoReq {
            cookies: req.cookies,
        };

        let resp = client.user_info(info_req).await
            .map_err(|e| Status::internal(format!("Failed to get user info: {}", e)))?;

        Ok(Response::new(UserInfoResponse {
            is_login: resp.is_login,
            username: resp.username,
            face: resp.face,
            is_vip: resp.is_vip,
        }))
    }

    async fn logout(&self, _request: Request<LogoutRequest>) -> Result<Response<LogoutResponse>, Status> {
        tracing::info!("gRPC Bilibili logout request");

        Ok(Response::new(LogoutResponse {
            message: "Logout successful".to_string(),
        }))
    }
}

/// Self-register Bilibili gRPC service on module load
pub fn init() {
    super::register_service_builder(|app_state, router| {
        tracing::info!("Registering Bilibili provider gRPC service");
        let service = BilibiliProviderGrpcService::new(app_state);
        router.add_service(BilibiliProviderServiceServer::new(service))
    });
}
