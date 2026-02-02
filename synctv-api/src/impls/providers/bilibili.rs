//! Bilibili API Implementation
//!
//! Unified implementation for all Bilibili API operations.
//! Used by both HTTP and gRPC handlers.

use std::sync::Arc;
use std::collections::HashMap;
use synctv_core::provider::BilibiliProvider;
use crate::proto::providers::bilibili::*;

/// Bilibili API implementation
///
/// Contains all business logic for Bilibili operations.
/// Methods accept grpc-generated request types and return grpc-generated response types.
#[derive(Clone)]
pub struct BilibiliApiImpl {
    provider: Arc<BilibiliProvider>,
}

impl BilibiliApiImpl {
    pub fn new(provider: Arc<BilibiliProvider>) -> Self {
        Self { provider }
    }

    /// Parse Bilibili URL
    pub async fn parse(&self, req: ParseRequest, instance_name: Option<&str>) -> Result<ParseResponse, String> {
        // Step 1: Match URL
        let match_resp = self.provider
            .r#match(req.url.clone(), instance_name)
            .await
            .map_err(|e| e.to_string())?;

        // Step 2: Parse based on type
        let page_info = match match_resp.r#type.as_str() {
            "video" | "bv" | "av" => {
                let parse_req = synctv_providers::grpc::bilibili::ParseVideoPageReq {
                    cookies: req.cookies.clone(),
                    bvid: if match_resp.r#type == "bv" { match_resp.id.clone() } else { String::new() },
                    aid: if match_resp.r#type == "av" { match_resp.id.parse().unwrap_or(0) } else { 0 },
                    sections: false,
                };

                self.provider
                    .parse_video_page(parse_req, instance_name)
                    .await
                    .map_err(|e| e.to_string())?
            }
            "pgc" | "ep" | "ss" => {
                let parse_req = synctv_providers::grpc::bilibili::ParsePgcPageReq {
                    cookies: req.cookies.clone(),
                    epid: if match_resp.r#type == "ep" { match_resp.id.parse().unwrap_or(0) } else { 0 },
                    ssid: if match_resp.r#type == "ss" { match_resp.id.parse().unwrap_or(0) } else { 0 },
                };

                self.provider
                    .parse_pgc_page(parse_req, instance_name)
                    .await
                    .map_err(|e| e.to_string())?
            }
            "live" => {
                let parse_req = synctv_providers::grpc::bilibili::ParseLivePageReq {
                    cookies: req.cookies.clone(),
                    room_id: match_resp.id.parse().unwrap_or(0),
                };

                self.provider
                    .parse_live_page(parse_req, instance_name)
                    .await
                    .map_err(|e| e.to_string())?
            }
            _ => {
                return Err(format!("Unsupported URL type: {}", match_resp.r#type));
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

        Ok(ParseResponse {
            title: page_info.title,
            actors,
            videos,
        })
    }

    /// Generate QR code for login
    pub async fn login_qr(&self, _req: LoginQrRequest, instance_name: Option<&str>) -> Result<QrCodeResponse, String> {
        let resp = self.provider
            .new_qr_code(instance_name)
            .await
            .map_err(|e| e.to_string())?;

        Ok(QrCodeResponse {
            url: resp.url,
            key: resp.key,
        })
    }

    /// Check QR code login status
    pub async fn check_qr(&self, req: CheckQrRequest, instance_name: Option<&str>) -> Result<QrStatusResponse, String> {
        let check_req = synctv_providers::grpc::bilibili::LoginWithQrCodeReq {
            key: req.key,
        };

        let resp = self.provider
            .login_with_qr_code(check_req, instance_name)
            .await
            .map_err(|e| e.to_string())?;

        Ok(QrStatusResponse {
            status: resp.status,
            cookies: resp.cookies,
        })
    }

    /// Get captcha for SMS login
    pub async fn get_captcha(&self, _req: GetCaptchaRequest, instance_name: Option<&str>) -> Result<CaptchaResponse, String> {
        let resp = self.provider
            .new_captcha(instance_name)
            .await
            .map_err(|e| e.to_string())?;

        Ok(CaptchaResponse {
            token: resp.token,
            gt: resp.gt,
            challenge: resp.challenge,
        })
    }

    /// Send SMS verification code
    pub async fn send_sms(&self, req: SendSmsRequest, instance_name: Option<&str>) -> Result<SendSmsResponse, String> {
        let sms_req = synctv_providers::grpc::bilibili::NewSmsReq {
            phone: req.phone,
            token: req.token,
            challenge: req.challenge,
            validate: req.validate,
        };

        let resp = self.provider
            .new_sms(sms_req, instance_name)
            .await
            .map_err(|e| e.to_string())?;

        Ok(SendSmsResponse {
            captcha_key: resp.captcha_key,
        })
    }

    /// Login with SMS code
    pub async fn login_sms(&self, req: LoginSmsRequest, instance_name: Option<&str>) -> Result<LoginSmsResponse, String> {
        let login_req = synctv_providers::grpc::bilibili::LoginWithSmsReq {
            phone: req.phone,
            code: req.code,
            captcha_key: req.captcha_key,
        };

        let resp = self.provider
            .login_with_sms(login_req, instance_name)
            .await
            .map_err(|e| e.to_string())?;

        Ok(LoginSmsResponse {
            cookies: resp.cookies,
        })
    }

    /// Get user info
    pub async fn get_user_info(&self, req: UserInfoRequest, instance_name: Option<&str>) -> Result<UserInfoResponse, String> {
        let info_req = synctv_providers::grpc::bilibili::UserInfoReq {
            cookies: req.cookies,
        };

        let resp = self.provider
            .user_info(info_req, instance_name)
            .await
            .map_err(|e| e.to_string())?;

        Ok(UserInfoResponse {
            is_login: resp.is_login,
            username: resp.username,
            face: resp.face,
            is_vip: resp.is_vip,
        })
    }

    /// Logout
    pub async fn logout(&self, _req: LogoutRequest) -> Result<LogoutResponse, String> {
        Ok(LogoutResponse {
            message: "Logout successful".to_string(),
        })
    }
}
