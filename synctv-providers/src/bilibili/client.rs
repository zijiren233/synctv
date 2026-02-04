//! Bilibili HTTP Client

use reqwest::Client;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use super::error::BilibiliError;
use super::types::{self as types, VideoInfo, Quality, PlayUrlInfo, DurlItem, AnimeInfo};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36";
const REFERER: &str = "https://www.bilibili.com";

/// Bilibili HTTP Client
pub struct BilibiliClient {
    client: Client,
    cookies: Option<HashMap<String, String>>,
}

impl BilibiliClient {
    /// Create a new Bilibili client
    pub fn new() -> Result<Self, BilibiliError> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| BilibiliError::Network(e.to_string()))?;

        Ok(Self {
            client,
            cookies: None,
        })
    }

    /// Create a new Bilibili client with cookies
    pub fn with_cookies(cookies: HashMap<String, String>) -> Result<Self, BilibiliError> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| BilibiliError::Network(e.to_string()))?;

        Ok(Self {
            client,
            cookies: Some(cookies),
        })
    }

    /// Add cookies to request
    fn add_cookies(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(cookies) = &self.cookies {
            let cookie_str = cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            req.header("Cookie", cookie_str)
        } else {
            req
        }
    }

    /// Generate QR code for login
    pub async fn new_qr_code(&self) -> Result<(String, String), BilibiliError> {
        #[derive(Deserialize)]
        struct QrCodeData {
            url: String,
            qrcode_key: String,
        }

        #[derive(Deserialize)]
        struct QrCodeResp {
            code: i32,
            message: String,
            data: Option<QrCodeData>,
        }

        let url = "https://passport.bilibili.com/x/passport-login/web/qrcode/generate";
        let req = self.client
            .get(url)
            .header("Referer", "https://passport.bilibili.com/login");

        let resp = req.send().await?;
        let json: QrCodeResp = resp.json().await?;

        if json.code != 0 {
            return Err(BilibiliError::Api(json.message));
        }

        let data = json.data.ok_or_else(|| BilibiliError::Parse("Missing QR code data".to_string()))?;
        Ok((data.url, data.qrcode_key))
    }

    /// Check QR code login status
    pub async fn login_with_qr_code(&self, key: &str) -> Result<(u32, Option<HashMap<String, String>>), BilibiliError> {
        #[derive(Deserialize)]
        struct LoginData {
            code: u32,
            #[allow(dead_code)]
            message: String,
        }

        #[derive(Deserialize)]
        struct LoginResp {
            code: i32,
            message: String,
            data: Option<LoginData>,
        }

        let url = format!("https://passport.bilibili.com/x/passport-login/web/qrcode/poll?qrcode_key={key}");
        let req = self.client
            .get(&url)
            .header("Referer", "https://passport.bilibili.com/login");

        let resp = req.send().await?;

        // Extract cookies
        let cookies = resp.cookies()
            .find(|c| c.name() == "SESSDATA")
            .map(|c| {
                let mut map = HashMap::new();
                map.insert(c.name().to_string(), c.value().to_string());
                map
            });

        let json: LoginResp = resp.json().await?;

        if json.code != 0 {
            return Err(BilibiliError::Api(json.message));
        }

        let data = json.data.ok_or_else(|| BilibiliError::Parse("Missing login data".to_string()))?;

        // QR code status codes:
        // 0: success
        // 86038: expired
        // 86090: scanned
        // 86101: not scanned
        Ok((data.code, cookies))
    }

    /// Get new captcha for SMS login
    pub async fn new_captcha(&self) -> Result<(String, String, String), BilibiliError> {
        #[derive(Deserialize)]
        struct Geetest {
            challenge: String,
            gt: String,
        }

        #[derive(Deserialize)]
        struct CaptchaData {
            token: String,
            geetest: Geetest,
        }

        #[derive(Deserialize)]
        struct CaptchaResp {
            code: i32,
            message: String,
            data: Option<CaptchaData>,
        }

        let url = "https://passport.bilibili.com/x/passport-login/captcha";
        let req = self.client
            .get(url)
            .header("Referer", "https://passport.bilibili.com/login");

        let resp = req.send().await?;
        let json: CaptchaResp = resp.json().await?;

        if json.code != 0 {
            return Err(BilibiliError::Api(json.message));
        }

        let data = json.data.ok_or_else(|| BilibiliError::Parse("Missing captcha data".to_string()))?;
        Ok((data.token, data.geetest.gt, data.geetest.challenge))
    }

    /// Get BUVID cookies for SMS operations
    async fn get_buvid_cookies(&self) -> Result<HashMap<String, String>, BilibiliError> {
        #[derive(Deserialize)]
        struct SpiData {
            #[serde(rename = "b_3")]
            b3: String,
            #[serde(rename = "b_4")]
            b4: String,
        }

        #[derive(Deserialize)]
        struct SpiResp {
            code: i32,
            message: String,
            data: Option<SpiData>,
        }

        let url = "https://api.bilibili.com/x/frontend/finger/spi";
        let req = self.client
            .get(url)
            .header("User-Agent", USER_AGENT)
            .header("Referer", "https://www.bilibili.com");

        let resp = req.send().await?;
        let json: SpiResp = resp.json().await?;

        if json.code != 0 {
            return Err(BilibiliError::Api(json.message));
        }

        let data = json.data.ok_or_else(|| BilibiliError::Parse("Missing BUVID data".to_string()))?;
        let mut cookies = HashMap::new();
        cookies.insert("buvid3".to_string(), data.b3);
        cookies.insert("buvid4".to_string(), data.b4);
        Ok(cookies)
    }

    /// Send SMS verification code
    pub async fn new_sms(
        &self,
        phone: &str,
        token: &str,
        challenge: &str,
        validate: &str,
    ) -> Result<String, BilibiliError> {
        #[derive(Deserialize)]
        struct SmsData {
            captcha_key: String,
        }

        #[derive(Deserialize)]
        struct SmsResp {
            code: i32,
            message: String,
            data: Option<SmsData>,
        }

        // Get BUVID cookies
        let buvid_cookies = self.get_buvid_cookies().await?;

        let seccode = format!("{validate}|jordan");
        let params = [
            ("cid", "86"),
            ("tel", phone),
            ("source", "main-fe-header"),
            ("token", token),
            ("challenge", challenge),
            ("validate", validate),
            ("seccode", &seccode),
        ];

        let url = "https://passport.bilibili.com/x/passport-login/web/sms/send";
        let mut req = self.client
            .post(url)
            .header("Referer", "https://passport.bilibili.com/login")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&params);

        // Add BUVID cookies
        for (name, value) in buvid_cookies {
            req = req.header("Cookie", format!("{name}={value}"));
        }

        let resp = req.send().await?;
        let json: SmsResp = resp.json().await?;

        if json.code != 0 {
            return Err(BilibiliError::Api(json.message));
        }

        let data = json.data.ok_or_else(|| BilibiliError::Parse("Missing SMS data".to_string()))?;
        Ok(data.captcha_key)
    }

    /// Login with SMS verification code
    pub async fn login_with_sms(
        &self,
        phone: &str,
        code: &str,
        captcha_key: &str,
    ) -> Result<HashMap<String, String>, BilibiliError> {
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct LoginSmsData {
            url: String,
            status: i32,
            is_new: bool,
        }

        #[derive(Deserialize)]
        struct LoginSmsResp {
            code: i32,
            message: String,
            #[allow(dead_code)]
            data: Option<LoginSmsData>,
        }

        let params = [
            ("cid", "86"),
            ("tel", phone),
            ("code", code),
            ("source", "main-fe-header"),
            ("captcha_key", captcha_key),
        ];

        let url = "https://passport.bilibili.com/x/passport-login/web/login/sms";
        let req = self.client
            .post(url)
            .header("Referer", "https://passport.bilibili.com/login")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&params);

        let resp = req.send().await?;

        // Extract cookies first (before consuming response body)
        let cookies: HashMap<String, String> = resp.cookies()
            .filter_map(|c| {
                if c.name() == "SESSDATA" {
                    Some((c.name().to_string(), c.value().to_string()))
                } else {
                    None
                }
            })
            .collect();

        let json: LoginSmsResp = resp.json().await?;

        if json.code != 0 {
            return Err(BilibiliError::Api(json.message));
        }

        if cookies.is_empty() {
            return Err(BilibiliError::Parse("No SESSDATA cookie found".to_string()));
        }

        Ok(cookies)
    }

    /// Extract BVID from URL
    #[must_use] 
    pub fn extract_bvid(url: &str) -> Option<String> {
        let re = Regex::new(r"BV[a-zA-Z0-9]+").unwrap();
        re.find(url).map(|m| m.as_str().to_string())
    }

    /// Extract EPID from URL
    #[must_use] 
    pub fn extract_epid(url: &str) -> Option<String> {
        let re = Regex::new(r"ep(\d+)").unwrap();
        re.captures(url).and_then(|cap| cap.get(1))
            .map(|m| format!("ep{}", m.as_str()))
    }

    /// Check if URL is a short link (b23.tv)
    #[must_use] 
    pub fn is_short_link(url: &str) -> bool {
        url.contains("b23.tv")
    }

    /// Resolve short link to full URL
    pub async fn resolve_short_link(&self, url: &str) -> Result<String, BilibiliError> {
        let response = self.client.get(url).send().await?;
        Ok(response.url().to_string())
    }

    /// Get video information by BVID
    pub async fn get_video_info(&self, bvid: &str) -> Result<VideoInfo, BilibiliError> {
        let url = format!("https://api.bilibili.com/x/web-interface/view?bvid={bvid}");
        let response = self.client.get(&url).send().await?;

        let json: serde_json::Value = response.json().await?;

        if json["code"].as_i64() != Some(0) {
            return Err(BilibiliError::Api(
                json["message"].as_str().unwrap_or("Unknown error").to_string()
            ));
        }

        let data = &json["data"];
        Ok(VideoInfo {
            bvid: data["bvid"].as_str().unwrap_or("").to_string(),
            aid: data["aid"].as_u64().unwrap_or(0),
            cid: data["cid"].as_u64().unwrap_or(0),
            title: data["title"].as_str().unwrap_or("").to_string(),
            desc: data["desc"].as_str().unwrap_or("").to_string(),
            pic: data["pic"].as_str().unwrap_or("").to_string(),
            duration: data["duration"].as_u64().unwrap_or(0),
        })
    }

    /// Get playback URL
    pub async fn get_play_url(
        &self,
        bvid: &str,
        cid: u64,
        quality: Quality,
    ) -> Result<PlayUrlInfo, BilibiliError> {
        let url = format!(
            "https://api.bilibili.com/x/player/playurl?bvid={}&cid={}&qn={}",
            bvid, cid, quality.to_qn()
        );

        let response = self.client.get(&url).send().await?;
        let json: serde_json::Value = response.json().await?;

        if json["code"].as_i64() != Some(0) {
            return Err(BilibiliError::Api(
                json["message"].as_str().unwrap_or("Unknown error").to_string()
            ));
        }

        let durl = json["data"]["durl"].as_array()
            .ok_or_else(|| BilibiliError::Parse("Missing durl array".to_string()))?
            .iter()
            .filter_map(|item| {
                Some(DurlItem {
                    url: item["url"].as_str()?.to_string(),
                    size: item["size"].as_u64().unwrap_or(0),
                })
            })
            .collect();

        Ok(PlayUrlInfo { durl })
    }

    /// Get anime information by EPID
    pub async fn get_anime_info(&self, epid: &str) -> Result<AnimeInfo, BilibiliError> {
        let url = format!("https://api.bilibili.com/pgc/view/web/season?ep_id={epid}");
        let response = self.client.get(&url).send().await?;
        let json: serde_json::Value = response.json().await?;

        if json["code"].as_i64() != Some(0) {
            return Err(BilibiliError::Api(
                json["message"].as_str().unwrap_or("Unknown error").to_string()
            ));
        }

        let data = &json["result"];
        Ok(AnimeInfo {
            season_id: data["season_id"].as_u64().unwrap_or(0),
            ep_id: data["episodes"][0]["ep_id"].as_u64().unwrap_or(0),
            cid: data["episodes"][0]["cid"].as_u64().unwrap_or(0),
            title: data["title"].as_str().unwrap_or("").to_string(),
            cover: data["cover"].as_str().unwrap_or("").to_string(),
        })
    }

    /// Parse video page to get video information
    pub async fn parse_video_page(&self, aid: u64, bvid: &str) -> Result<VideoPageInfo, BilibiliError> {
        let url = if bvid.is_empty() {
            format!("https://api.bilibili.com/x/web-interface/view?aid={aid}")
        } else {
            format!("https://api.bilibili.com/x/web-interface/view?bvid={bvid}")
        };

        let req = self.add_cookies(self.client.get(&url).header("Referer", REFERER));
        let resp = req.send().await?;
        let json: types::VideoPageInfoResp = resp.json().await?;

        if json.code != 0 {
            return Err(BilibiliError::Api(json.message));
        }

        let data = json.data;
        let title = data.title;
        let owner_name = data.owner.name;

        let mut video_infos = Vec::new();
        for page in data.pages {
            video_infos.push(VideoInfoItem {
                bvid: data.bvid.clone(),
                cid: page.cid,
                epid: 0,
                name: page.part,
                cover_image: data.pic.clone(),
                live: false,
            });
        }

        Ok(VideoPageInfo {
            title,
            actors: vec![owner_name],
            video_infos,
        })
    }

    /// Get video playback URL (normal video, not DASH)
    pub async fn get_video_url(&self, aid: u64, bvid: &str, cid: u64, quality: Option<u32>) -> Result<VideoUrlInfo, BilibiliError> {
        let qn = quality.unwrap_or(80); // Default to 1080P
        let url = if bvid.is_empty() {
            format!("https://api.bilibili.com/x/player/playurl?aid={aid}&cid={cid}&qn={qn}")
        } else {
            format!("https://api.bilibili.com/x/player/playurl?bvid={bvid}&cid={cid}&qn={qn}")
        };

        let req = self.add_cookies(self.client.get(&url).header("Referer", REFERER));
        let resp = req.send().await?;
        let json: types::VideoUrlResp = resp.json().await?;

        if json.code != 0 {
            return Err(BilibiliError::Api(json.message));
        }

        let data = json.data;
        let accept_quality: Vec<u32> = data.accept_quality.iter().map(|&q| q as u32).collect();
        let accept_description = data.accept_description;
        let current_quality = data.quality as u32;
        let url = data.durl.first()
            .map(|d| d.url.clone())
            .unwrap_or_default();

        Ok(VideoUrlInfo {
            accept_quality,
            accept_description,
            current_quality,
            url,
        })
    }

    /// Get DASH video URL - returns structured DASH data for upper layer to generate MPD
    pub async fn get_dash_video_url(&self, aid: u64, bvid: &str, cid: u64) -> Result<(DashData, DashData), BilibiliError> {
        let url = if bvid.is_empty() {
            format!("https://api.bilibili.com/x/player/wbi/playurl?aid={aid}&cid={cid}&fnval=4048")
        } else {
            format!("https://api.bilibili.com/x/player/wbi/playurl?bvid={bvid}&cid={cid}&fnval=4048")
        };

        let req = self.add_cookies(self.client.get(&url).header("Referer", REFERER));
        let resp = req.send().await?;
        let json: types::DashVideoResp = resp.json().await?;

        if json.code != 0 {
            return Err(BilibiliError::Api(json.message));
        }

        // Parse DASH data into structured format
        let dash_info = json.data.dash;
        let (regular_dash, hevc_dash) = parse_dash_info(&dash_info)?;

        Ok((regular_dash, hevc_dash))
    }

    /// Get subtitles for a video
    pub async fn get_subtitles(&self, aid: u64, bvid: &str, cid: u64) -> Result<HashMap<String, String>, BilibiliError> {
        let url = if bvid.is_empty() {
            format!("https://api.bilibili.com/x/player/v2?aid={aid}&cid={cid}")
        } else {
            format!("https://api.bilibili.com/x/player/v2?bvid={bvid}&cid={cid}")
        };

        let req = self.add_cookies(self.client.get(&url).header("Referer", REFERER));
        let resp = req.send().await?;
        let json: types::PlayerV2InfoResp = resp.json().await?;

        if json.code != 0 {
            return Err(BilibiliError::Api(json.message));
        }

        let mut subtitles = HashMap::new();
        for sub in json.data.subtitle.subtitles {
            let name = sub.lan_doc;
            let url = if sub.subtitle_url.starts_with("http") {
                sub.subtitle_url
            } else {
                format!("https:{}", sub.subtitle_url)
            };
            if !name.is_empty() && !url.is_empty() {
                subtitles.insert(name, url);
            }
        }

        Ok(subtitles)
    }

    /// Get user information
    pub async fn user_info(&self) -> Result<UserInfo, BilibiliError> {
        let url = "https://api.bilibili.com/x/web-interface/nav";
        let req = self.add_cookies(self.client.get(url).header("Referer", REFERER));
        let resp = req.send().await?;
        let json: types::NavResp = resp.json().await?;

        if json.code != 0 {
            return Err(BilibiliError::Api(json.message));
        }

        let data = json.data;
        Ok(UserInfo {
            is_login: data.is_login,
            username: data.uname,
            face: data.face,
            is_vip: data.vip_status == 1,
        })
    }

    /// Parse PGC (anime/bangumi) page
    pub async fn parse_pgc_page(&self, epid: u64, ssid: u64) -> Result<VideoPageInfo, BilibiliError> {
        let url = if epid != 0 {
            format!("https://api.bilibili.com/pgc/view/web/season?ep_id={epid}")
        } else {
            format!("https://api.bilibili.com/pgc/view/web/season?season_id={ssid}")
        };

        let req = self.add_cookies(self.client.get(&url).header("Referer", REFERER));
        let resp = req.send().await?;
        let json: types::SeasonInfoResp = resp.json().await?;

        if json.code != 0 {
            return Err(BilibiliError::Api(json.message));
        }

        let result = json.result;
        let title = result.title;
        let actors_str = result.actors;
        let actors = if actors_str.is_empty() {
            vec![]
        } else {
            vec![actors_str]
        };

        let mut video_infos = Vec::new();
        for ep in result.episodes {
            video_infos.push(VideoInfoItem {
                bvid: ep.bvid,
                cid: ep.cid,
                epid: ep.ep_id,
                name: if ep.long_title.is_empty() { ep.title } else { ep.long_title },
                cover_image: ep.cover,
                live: false,
            });
        }

        Ok(VideoPageInfo {
            title,
            actors,
            video_infos,
        })
    }

    /// Get PGC playback URL
    pub async fn get_pgc_url(&self, epid: u64, cid: u64, quality: Option<u32>) -> Result<VideoUrlInfo, BilibiliError> {
        let qn = quality.unwrap_or(80);
        let url = format!(
            "https://api.bilibili.com/pgc/player/web/playurl?ep_id={epid}&cid={cid}&qn={qn}"
        );

        let req = self.add_cookies(self.client.get(&url).header("Referer", REFERER));
        let resp = req.send().await?;
        let json: types::PgcUrlResp = resp.json().await?;

        if json.code != 0 {
            return Err(BilibiliError::Api(json.message));
        }

        let result = json.result;
        let accept_quality: Vec<u32> = result.accept_quality.iter().map(|&q| q as u32).collect();
        let accept_description = result.accept_description;
        let current_quality = result.quality as u32;
        let url = result.durl.first()
            .map(|d| d.url.clone())
            .unwrap_or_default();

        Ok(VideoUrlInfo {
            accept_quality,
            accept_description,
            current_quality,
            url,
        })
    }

    /// Get DASH PGC URL - returns structured DASH data for upper layer to generate MPD
    pub async fn get_dash_pgc_url(&self, epid: u64, cid: u64) -> Result<(DashData, DashData), BilibiliError> {
        let url = format!(
            "https://api.bilibili.com/pgc/player/web/playurl?ep_id={epid}&cid={cid}&fnval=4048"
        );

        let req = self.add_cookies(self.client.get(&url).header("Referer", REFERER));
        let resp = req.send().await?;
        let json: types::DashPgcResp = resp.json().await?;

        if json.code != 0 {
            return Err(BilibiliError::Api(json.message));
        }

        // Parse DASH data into structured format
        let dash_info = json.result.dash;
        let (regular_dash, hevc_dash) = parse_dash_info(&dash_info)?;

        Ok((regular_dash, hevc_dash))
    }

    /// Match URL to extract video type and ID
    pub fn match_url(url: &str) -> Result<(String, String), BilibiliError> {
        // Video: BV id
        if let Some(bvid) = Self::extract_bvid(url) {
            return Ok(("video".to_string(), bvid));
        }

        // Bangumi/Anime: ep id or ss id
        if url.contains("/bangumi/play/") {
            if let Some(ep_match) = Regex::new(r"ep(\d+)").unwrap().captures(url) {
                if let Some(ep_id) = ep_match.get(1) {
                    return Ok(("bangumi".to_string(), format!("ep{}", ep_id.as_str())));
                }
            }
            if let Some(ss_match) = Regex::new(r"ss(\d+)").unwrap().captures(url) {
                if let Some(ss_id) = ss_match.get(1) {
                    return Ok(("bangumi".to_string(), format!("ss{}", ss_id.as_str())));
                }
            }
        }

        // Live: room id
        if url.contains("/live/") || url.contains("live.bilibili.com") {
            if let Some(room_match) = Regex::new(r"/live/(\d+)").unwrap().captures(url) {
                if let Some(room_id) = room_match.get(1) {
                    return Ok(("live".to_string(), room_id.as_str().to_string()));
                }
            }
        }

        Err(BilibiliError::Parse("Cannot parse URL type".to_string()))
    }

    /// Parse live page
    pub async fn parse_live_page(&self, room_id: u64) -> Result<VideoPageInfo, BilibiliError> {
        let url = format!("https://api.live.bilibili.com/room/v1/Room/get_info?room_id={room_id}");

        let req = self.add_cookies(self.client.get(&url).header("Referer", REFERER));
        let resp = req.send().await?;
        let json: types::ParseLivePageResp = resp.json().await?;

        if json.code != 0 {
            return Err(BilibiliError::Api(json.message));
        }

        let data = json.data;
        let title = data.title.clone();

        // Note: This API doesn't return uname directly, need to call get_live_master_info separately
        let uname = String::new();

        let video_info = VideoInfoItem {
            bvid: String::new(),
            cid: room_id,
            epid: 0,
            name: title.clone(),
            cover_image: data.user_cover,
            live: true,
        };

        Ok(VideoPageInfo {
            title,
            actors: vec![uname],
            video_infos: vec![video_info],
        })
    }

    /// Get live streams
    pub async fn get_live_streams(&self, room_id: u64, hls: bool) -> Result<Vec<LiveStream>, BilibiliError> {
        let _protocol = i32::from(!hls); // 0: http_stream (FLV), 1: http_hls (HLS)
        let url = format!(
            "https://api.live.bilibili.com/xlive/web-room/v2/index/getRoomPlayInfo?room_id={room_id}&protocol=0,1&format=0,1,2&codec=0,1&qn=10000&platform=web&ptype=8"
        );

        let req = self.add_cookies(self.client.get(&url).header("Referer", REFERER));
        let resp = req.send().await?;
        let json: serde_json::Value = resp.json().await?;

        if json["code"].as_i64() != Some(0) {
            return Err(BilibiliError::Api(
                json["message"].as_str().unwrap_or("Unknown error").to_string()
            ));
        }

        let mut streams = Vec::new();

        if let Some(stream_list) = json["data"]["playurl_info"]["playurl"]["stream"].as_array() {
            for stream in stream_list {
                if let Some(format_list) = stream["format"].as_array() {
                    for format in format_list {
                        if let Some(codec_list) = format["codec"].as_array() {
                            for codec in codec_list {
                                let quality = codec["current_qn"].as_u64().unwrap_or(0) as u32;
                                let desc = codec["accept_qn"].as_array()
                                    .and_then(|arr| arr.first())
                                    .and_then(serde_json::Value::as_u64).map_or_else(|| "Unknown".to_string(), |q| format!("{q}P"));

                                let urls: Vec<String> = codec["url_info"].as_array()
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|item| {
                                                let host = item["host"].as_str()?;
                                                let path = codec["base_url"].as_str()?;
                                                Some(format!("{host}{path}"))
                                            })
                                            .collect()
                                    })
                                    .unwrap_or_default();

                                if !urls.is_empty() {
                                    streams.push(LiveStream {
                                        quality,
                                        urls,
                                        desc,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(streams)
    }

    /// Get live danmaku server info
    pub async fn get_live_danmu_info(&self, room_id: u64) -> Result<LiveDanmuInfo, BilibiliError> {
        let url = format!("https://api.live.bilibili.com/xlive/web-room/v1/index/getDanmuInfo?id={room_id}");

        let req = self.add_cookies(self.client.get(&url).header("Referer", REFERER));
        let resp = req.send().await?;
        let json: types::GetLiveDanmuInfoResp = resp.json().await?;

        if json.code != 0 {
            return Err(BilibiliError::Api(json.message));
        }

        let data = json.data;
        let token = data.token;
        let host_list: Vec<DanmuHost> = data.host_list
            .into_iter()
            .map(|h| DanmuHost {
                host: h.host,
                port: h.port,
                wss_port: h.wss_port,
                ws_port: h.ws_port,
            })
            .collect();

        Ok(LiveDanmuInfo {
            token,
            host_list,
        })
    }
}

/// Video page information
#[derive(Debug, Clone)]
pub struct VideoPageInfo {
    pub title: String,
    pub actors: Vec<String>,
    pub video_infos: Vec<VideoInfoItem>,
}

#[derive(Debug, Clone)]
pub struct VideoInfoItem {
    pub bvid: String,
    pub cid: u64,
    pub epid: u64,
    pub name: String,
    pub cover_image: String,
    pub live: bool,
}

/// Video URL information
#[derive(Debug, Clone)]
pub struct VideoUrlInfo {
    pub accept_quality: Vec<u32>,
    pub accept_description: Vec<String>,
    pub current_quality: u32,
    pub url: String,
}

/// User information
#[derive(Debug, Clone)]
pub struct UserInfo {
    pub is_login: bool,
    pub username: String,
    pub face: String,
    pub is_vip: bool,
}

/// Live stream information
#[derive(Debug, Clone)]
pub struct LiveStream {
    pub quality: u32,
    pub urls: Vec<String>,
    pub desc: String,
}

/// Live danmaku server information
#[derive(Debug, Clone)]
pub struct LiveDanmuInfo {
    pub token: String,
    pub host_list: Vec<DanmuHost>,
}

/// Danmaku server host
#[derive(Debug, Clone)]
pub struct DanmuHost {
    pub host: String,
    pub port: u32,
    pub wss_port: u32,
    pub ws_port: u32,
}

impl Default for BilibiliClient {
    fn default() -> Self {
        Self::new().unwrap()
    }
}

/// DASH stream data (structured for upper layer to generate MPD)
#[derive(Debug, Clone)]
pub struct DashData {
    pub duration: f64,
    pub min_buffer_time: f64,
    pub video_streams: Vec<VideoStreamData>,
    pub audio_streams: Vec<AudioStreamData>,
}

/// Video stream representation
#[derive(Debug, Clone)]
pub struct VideoStreamData {
    pub id: u64,
    pub base_url: String,
    pub mime_type: String,
    pub codecs: String,
    pub width: u64,
    pub height: u64,
    pub frame_rate: String,
    pub bandwidth: u64,
    pub start_with_sap: u64,
    pub segment_base: SegmentBaseData,
}

/// Audio stream representation
#[derive(Debug, Clone)]
pub struct AudioStreamData {
    pub id: u64,
    pub base_url: String,
    pub mime_type: String,
    pub codecs: String,
    pub bandwidth: u64,
    pub start_with_sap: u64,
    pub segment_base: SegmentBaseData,
}

/// Segment base information
#[derive(Debug, Clone)]
pub struct SegmentBaseData {
    pub index_range: String,
    pub initialization_range: String,
}

// ============================================================================
// From trait implementations for proto conversion
// ============================================================================

impl From<&SegmentBaseData> for crate::grpc::bilibili::SegmentBase {
    fn from(data: &SegmentBaseData) -> Self {
        Self {
            index_range: data.index_range.clone(),
            initialization_range: data.initialization_range.clone(),
        }
    }
}

impl From<&VideoStreamData> for crate::grpc::bilibili::VideoStream {
    fn from(data: &VideoStreamData) -> Self {
        Self {
            id: data.id,
            base_url: data.base_url.clone(),
            mime_type: data.mime_type.clone(),
            codecs: data.codecs.clone(),
            width: data.width,
            height: data.height,
            frame_rate: data.frame_rate.clone(),
            bandwidth: data.bandwidth,
            start_with_sap: data.start_with_sap,
            segment_base: Some((&data.segment_base).into()),
        }
    }
}

impl From<&AudioStreamData> for crate::grpc::bilibili::AudioStream {
    fn from(data: &AudioStreamData) -> Self {
        Self {
            id: data.id,
            base_url: data.base_url.clone(),
            mime_type: data.mime_type.clone(),
            codecs: data.codecs.clone(),
            bandwidth: data.bandwidth,
            start_with_sap: data.start_with_sap,
            segment_base: Some((&data.segment_base).into()),
        }
    }
}

impl From<&DashData> for crate::grpc::bilibili::DashInfo {
    fn from(data: &DashData) -> Self {
        Self {
            duration: data.duration,
            min_buffer_time: data.min_buffer_time,
            video_streams: data.video_streams.iter().map(std::convert::Into::into).collect(),
            audio_streams: data.audio_streams.iter().map(std::convert::Into::into).collect(),
        }
    }
}

/// Parse DASH info into structured format
/// Returns (`regular_dash`, `hevc_dash`) where HEVC codecs are separated
fn parse_dash_info(dash_info: &types::DashInfo) -> Result<(DashData, DashData), BilibiliError> {
    let duration = dash_info.duration;
    let min_buffer_time = dash_info.min_buffer_time;

    // Parse audio streams (shared by both regular and HEVC)
    let parsed_audios: Vec<AudioStreamData> = dash_info.audio
        .iter()
        .map(|audio| AudioStreamData {
            id: audio.id,
            base_url: audio.base_url.clone(),
            mime_type: audio.mime_type.clone(),
            codecs: audio.codecs.clone(),
            bandwidth: audio.bandwidth,
            start_with_sap: audio.start_with_sap,
            segment_base: SegmentBaseData {
                index_range: audio.segment_base.index_range.clone(),
                initialization_range: audio.segment_base.initialization.clone(),
            },
        })
        .collect();

    // Separate videos into regular and HEVC
    let mut regular_videos = Vec::new();
    let mut hevc_videos = Vec::new();

    for video in &dash_info.video {
        let video_data = VideoStreamData {
            id: video.id,
            base_url: video.base_url.clone(),
            mime_type: video.mime_type.clone(),
            codecs: video.codecs.clone(),
            width: video.width,
            height: video.height,
            frame_rate: video.frame_rate.clone(),
            bandwidth: video.bandwidth,
            start_with_sap: video.start_with_sap,
            segment_base: SegmentBaseData {
                index_range: video.segment_base.index_range.clone(),
                initialization_range: video.segment_base.initialization.clone(),
            },
        };

        if video_data.codecs.starts_with("hev1") || video_data.codecs.starts_with("hvc1") {
            hevc_videos.push(video_data);
        } else {
            regular_videos.push(video_data);
        }
    }

    let regular_dash = DashData {
        duration,
        min_buffer_time,
        video_streams: regular_videos,
        audio_streams: parsed_audios.clone(),
    };

    let hevc_dash = DashData {
        duration,
        min_buffer_time,
        video_streams: hevc_videos,
        audio_streams: parsed_audios,
    };

    Ok((regular_dash, hevc_dash))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_bvid() {
        assert_eq!(
            BilibiliClient::extract_bvid("https://www.bilibili.com/video/BV1xx411c7XZ"),
            Some("BV1xx411c7XZ".to_string())
        );
    }

    #[test]
    fn test_extract_epid() {
        assert_eq!(
            BilibiliClient::extract_epid("https://www.bilibili.com/bangumi/play/ep12345"),
            Some("ep12345".to_string())
        );
    }

    #[test]
    fn test_is_short_link() {
        assert!(BilibiliClient::is_short_link("https://b23.tv/abc123"));
        assert!(!BilibiliClient::is_short_link("https://www.bilibili.com/video/BV123"));
    }

    #[test]
    fn test_quality_conversion() {
        assert_eq!(Quality::P1080.to_qn(), 80);
        assert_eq!(Quality::from_qn(64), Quality::P720);
        assert_eq!(Quality::P480.as_str(), "480P");
    }
}
