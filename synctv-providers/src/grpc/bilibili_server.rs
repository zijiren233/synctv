//! Bilibili gRPC Server Implementation
//!
//! Thin wrapper around BilibiliService that implements gRPC server trait.

use super::bilibili::{
    bilibili_server::Bilibili, Empty, GetDashPgcurlReq, GetDashPgcurlResp, GetDashVideoUrlReq,
    GetDashVideoUrlResp, GetLiveDanmuInfoReq, GetLiveDanmuInfoResp, GetLiveStreamsReq,
    GetLiveStreamsResp, GetPgcurlReq, GetSubtitlesReq, GetSubtitlesResp, GetVideoUrlReq,
    LoginWithQrCodeReq, LoginWithQrCodeResp, LoginWithSmsReq, LoginWithSmsResp, MatchReq,
    MatchResp, NewCaptchaResp, NewQrCodeResp, NewSmsReq, NewSmsResp, ParseLivePageReq,
    ParsePgcPageReq, ParseVideoPageReq, UserInfoReq, UserInfoResp, VideoPageInfo, VideoUrl,
};
use crate::bilibili::{BilibiliInterface, BilibiliService as BilibiliServiceImpl};
use tonic::{Request, Response, Status};

/// Bilibili gRPC server
///
/// Thin wrapper that delegates to BilibiliService for actual implementation.
pub struct BilibiliService {
    service: BilibiliServiceImpl,
}

impl BilibiliService {
    pub fn new() -> Self {
        Self {
            service: BilibiliServiceImpl::new(),
        }
    }
}

impl Default for BilibiliService {
    fn default() -> Self {
        Self::new()
    }
}

#[tonic::async_trait]
impl Bilibili for BilibiliService {
    async fn new_qr_code(&self, request: Request<Empty>) -> Result<Response<NewQrCodeResp>, Status> {
        let req = request.into_inner();
        let resp = self.service.new_qr_code(req).await
            .map_err(|e| Status::internal(format!("new_qr_code failed: {}", e)))?;
        Ok(Response::new(resp))
    }

    async fn login_with_qr_code(
        &self,
        request: Request<LoginWithQrCodeReq>,
    ) -> Result<Response<LoginWithQrCodeResp>, Status> {
        let req = request.into_inner();
        let resp = self.service.login_with_qr_code(req).await
            .map_err(|e| Status::internal(format!("login_with_qr_code failed: {}", e)))?;
        Ok(Response::new(resp))
    }

    async fn new_captcha(
        &self,
        request: Request<Empty>,
    ) -> Result<Response<NewCaptchaResp>, Status> {
        let req = request.into_inner();
        let resp = self.service.new_captcha(req).await
            .map_err(|e| Status::internal(format!("new_captcha failed: {}", e)))?;
        Ok(Response::new(resp))
    }

    async fn new_sms(&self, request: Request<NewSmsReq>) -> Result<Response<NewSmsResp>, Status> {
        let req = request.into_inner();
        let resp = self.service.new_sms(req).await
            .map_err(|e| Status::internal(format!("new_sms failed: {}", e)))?;
        Ok(Response::new(resp))
    }

    async fn login_with_sms(
        &self,
        request: Request<LoginWithSmsReq>,
    ) -> Result<Response<LoginWithSmsResp>, Status> {
        let req = request.into_inner();
        let resp = self.service.login_with_sms(req).await
            .map_err(|e| Status::internal(format!("login_with_sms failed: {}", e)))?;
        Ok(Response::new(resp))
    }

    async fn parse_video_page(
        &self,
        request: Request<ParseVideoPageReq>,
    ) -> Result<Response<VideoPageInfo>, Status> {
        let req = request.into_inner();
        let resp = self.service.parse_video_page(req).await
            .map_err(|e| Status::internal(format!("parse_video_page failed: {}", e)))?;
        Ok(Response::new(resp))
    }

    async fn get_video_url(
        &self,
        request: Request<GetVideoUrlReq>,
    ) -> Result<Response<VideoUrl>, Status> {
        let req = request.into_inner();
        let resp = self.service.get_video_url(req).await
            .map_err(|e| Status::internal(format!("get_video_url failed: {}", e)))?;
        Ok(Response::new(resp))
    }

    async fn get_dash_video_url(
        &self,
        request: Request<GetDashVideoUrlReq>,
    ) -> Result<Response<GetDashVideoUrlResp>, Status> {
        let req = request.into_inner();
        let resp = self.service.get_dash_video_url(req).await
            .map_err(|e| Status::internal(format!("get_dash_video_url failed: {}", e)))?;
        Ok(Response::new(resp))
    }

    async fn get_subtitles(
        &self,
        request: Request<GetSubtitlesReq>,
    ) -> Result<Response<GetSubtitlesResp>, Status> {
        let req = request.into_inner();
        let resp = self.service.get_subtitles(req).await
            .map_err(|e| Status::internal(format!("get_subtitles failed: {}", e)))?;
        Ok(Response::new(resp))
    }

    async fn parse_pgc_page(
        &self,
        request: Request<ParsePgcPageReq>,
    ) -> Result<Response<VideoPageInfo>, Status> {
        let req = request.into_inner();
        let resp = self.service.parse_pgc_page(req).await
            .map_err(|e| Status::internal(format!("parse_pgc_page failed: {}", e)))?;
        Ok(Response::new(resp))
    }

    async fn get_pgcurl(
        &self,
        request: Request<GetPgcurlReq>,
    ) -> Result<Response<VideoUrl>, Status> {
        let req = request.into_inner();
        let resp = self.service.get_pgcurl(req).await
            .map_err(|e| Status::internal(format!("get_pgcurl failed: {}", e)))?;
        Ok(Response::new(resp))
    }

    async fn get_dash_pgcurl(
        &self,
        request: Request<GetDashPgcurlReq>,
    ) -> Result<Response<GetDashPgcurlResp>, Status> {
        let req = request.into_inner();
        let resp = self.service.get_dash_pgcurl(req).await
            .map_err(|e| Status::internal(format!("get_dash_pgcurl failed: {}", e)))?;
        Ok(Response::new(resp))
    }

    async fn user_info(
        &self,
        request: Request<UserInfoReq>,
    ) -> Result<Response<UserInfoResp>, Status> {
        let req = request.into_inner();
        let resp = self.service.user_info(req).await
            .map_err(|e| Status::internal(format!("user_info failed: {}", e)))?;
        Ok(Response::new(resp))
    }

    async fn r#match(&self, request: Request<MatchReq>) -> Result<Response<MatchResp>, Status> {
        let req = request.into_inner();
        let resp = self.service.r#match(req).await
            .map_err(|e| Status::internal(format!("match failed: {}", e)))?;
        Ok(Response::new(resp))
    }

    async fn get_live_streams(
        &self,
        request: Request<GetLiveStreamsReq>,
    ) -> Result<Response<GetLiveStreamsResp>, Status> {
        let req = request.into_inner();
        let resp = self.service.get_live_streams(req).await
            .map_err(|e| Status::internal(format!("get_live_streams failed: {}", e)))?;
        Ok(Response::new(resp))
    }

    async fn parse_live_page(
        &self,
        request: Request<ParseLivePageReq>,
    ) -> Result<Response<VideoPageInfo>, Status> {
        let req = request.into_inner();
        let resp = self.service.parse_live_page(req).await
            .map_err(|e| Status::internal(format!("parse_live_page failed: {}", e)))?;
        Ok(Response::new(resp))
    }

    async fn get_live_danmu_info(
        &self,
        request: Request<GetLiveDanmuInfoReq>,
    ) -> Result<Response<GetLiveDanmuInfoResp>, Status> {
        let req = request.into_inner();
        let resp = self.service.get_live_danmu_info(req).await
            .map_err(|e| Status::internal(format!("get_live_danmu_info failed: {}", e)))?;
        Ok(Response::new(resp))
    }
}
