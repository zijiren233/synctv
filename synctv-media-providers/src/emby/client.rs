//! Emby/Jellyfin HTTP Client

use std::sync::LazyLock;
use std::time::Duration;

use reqwest::{Client, header::{HeaderMap, HeaderValue, CONTENT_TYPE}};
use serde_json::{json, Value};

use super::error::{EmbyError, check_response, json_with_limit};
use super::types::{AuthResponse, Item, UserInfo, ItemsResponse, SystemInfo, FsListResponse, PathInfo, PlaybackInfoResponse, default_device_profile};

/// URL-encode a string for safe use in query parameters
fn url_encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// Shared HTTP client for all Emby requests (connection pooling)
/// Redirects are disabled to prevent SSRF via redirect to private IPs.
static SHARED_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(10)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("Failed to build Emby shared HTTP client")
});

const X_EMBY_TOKEN: &str = "X-Emby-Token";

/// Emby/Jellyfin HTTP Client
pub struct EmbyClient {
    host: String,
    token: Option<String>,
    user_id: Option<String>,
    client: Client,
    api_prefix: Option<String>,
}

impl EmbyClient {
    /// Create a new Emby client (reuses shared connection pool)
    pub fn new(host: impl Into<String>) -> Result<Self, EmbyError> {
        Ok(Self {
            host: host.into(),
            token: None,
            user_id: None,
            client: SHARED_CLIENT.clone(),
            api_prefix: None,
        })
    }

    /// Create a new Emby client with credentials (reuses shared connection pool)
    pub fn with_credentials(
        host: impl Into<String>,
        token: impl Into<String>,
        user_id: impl Into<String>,
    ) -> Result<Self, EmbyError> {
        Ok(Self {
            host: host.into(),
            token: Some(token.into()),
            user_id: Some(user_id.into()),
            client: SHARED_CLIENT.clone(),
            api_prefix: None,
        })
    }

    /// Set a custom API prefix (e.g., "/emby" or "/jellyfin").
    /// When set, overrides the auto-detection based on hostname.
    pub fn set_api_prefix(&mut self, prefix: impl Into<String>) {
        self.api_prefix = Some(prefix.into());
    }

    /// Set authentication token and user ID
    pub fn set_credentials(&mut self, token: impl Into<String>, user_id: impl Into<String>) {
        self.token = Some(token.into());
        self.user_id = Some(user_id.into());
    }

    /// Get API prefix (/emby or /jellyfin).
    /// Uses the explicitly set prefix if available, otherwise auto-detects
    /// based on whether the host URL contains "jellyfin".
    fn get_api_prefix(&self) -> &str {
        if let Some(ref prefix) = self.api_prefix {
            return prefix;
        }
        if self.host.contains("jellyfin") {
            "/jellyfin"
        } else {
            "/emby"
        }
    }

    /// Build request headers
    fn build_headers(&self) -> Result<HeaderMap, EmbyError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        if let Some(ref token) = self.token {
            headers.insert(X_EMBY_TOKEN, HeaderValue::from_str(token)?);
        }

        Ok(headers)
    }

    /// Login to Emby/Jellyfin server
    pub async fn login(&mut self, username: &str, password: &str) -> Result<(String, String), EmbyError> {
        let prefix = self.get_api_prefix();
        let url = format!("{}{}/Users/authenticatebyname", self.host, prefix);

        let body = json!({
            "Username": username,
            "Pw": password,
        });

        let response = self
            .client
            .post(&url)
            .headers(self.build_headers()?)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(EmbyError::Auth(format!("Login failed: {}", response.status())));
        }

        let auth_resp: AuthResponse = json_with_limit(response).await?;
        let token = auth_resp.access_token;
        let user_id = auth_resp.user.id;

        self.set_credentials(token.clone(), user_id.clone());
        Ok((token, user_id))
    }

    /// Get item information
    pub async fn get_item(&self, item_id: &str) -> Result<Item, EmbyError> {
        let prefix = self.get_api_prefix();
        let url = format!("{}{}/Users/{}/Items?Ids={}",
            self.host,
            prefix,
            url_encode(self.user_id.as_ref().ok_or_else(|| EmbyError::InvalidConfig("Missing user_id".to_string()))?),
            url_encode(item_id)
        );

        let response = self
            .client
            .get(&url)
            .headers(self.build_headers()?)
            .send()
            .await?;

        let response = check_response(response)?;
        let json: Value = json_with_limit(response).await?;
        let items = json["Items"].as_array()
            .ok_or_else(|| EmbyError::Parse("Missing Items array".to_string()))?;

        if items.is_empty() {
            return Err(EmbyError::Api { code: 0, message: "Item not found".to_string() });
        }

        let item: Item = serde_json::from_value(items[0].clone())?;
        Ok(item)
    }

    /// Get current user information
    pub async fn me(&self) -> Result<UserInfo, EmbyError> {
        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| EmbyError::InvalidConfig("Missing user_id".to_string()))?;

        let prefix = self.get_api_prefix();
        let url = format!("{}{}/Users/{}", self.host, prefix, url_encode(user_id));

        let response = self
            .client
            .get(&url)
            .headers(self.build_headers()?)
            .send()
            .await?;

        let response = check_response(response)?;
        let user: UserInfo = json_with_limit(response).await?;
        Ok(user)
    }

    /// Get items list
    pub async fn get_items(
        &self,
        parent_id: Option<&str>,
        search_term: Option<&str>,
    ) -> Result<ItemsResponse, EmbyError> {
        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| EmbyError::InvalidConfig("Missing user_id".to_string()))?;

        let prefix = self.get_api_prefix();
        let mut url = format!("{}{}/Users/{}/Items?SortBy=SortName&SortOrder=Ascending",
            self.host, prefix, url_encode(user_id));

        if let Some(pid) = parent_id {
            url.push_str(&format!("&ParentId={}", url_encode(pid)));
        }

        if let Some(term) = search_term {
            url.push_str(&format!("&SearchTerm={}&Recursive=true", url_encode(term)));
        } else {
            url.push_str("&Filters=IsNotFolder");
        }

        let response = self
            .client
            .get(&url)
            .headers(self.build_headers()?)
            .send()
            .await?;

        let response = check_response(response)?;
        let items: ItemsResponse = json_with_limit(response).await?;
        Ok(items)
    }

    /// Get system information
    pub async fn get_system_info(&self) -> Result<SystemInfo, EmbyError> {
        let prefix = self.get_api_prefix();
        let url = format!("{}{}/System/Info", self.host, prefix);

        let response = self
            .client
            .get(&url)
            .headers(self.build_headers()?)
            .send()
            .await?;

        let response = check_response(response)?;
        let info: SystemInfo = json_with_limit(response).await?;
        Ok(info)
    }

    /// Filesystem list
    pub async fn fs_list(
        &self,
        path: Option<&str>,
        start_index: u64,
        limit: u64,
        search_term: Option<&str>,
    ) -> Result<FsListResponse, EmbyError> {
        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| EmbyError::InvalidConfig("Missing user_id".to_string()))?;

        let prefix = self.get_api_prefix();

        // Get user views (libraries) if no path specified
        if path.is_none() && search_term.is_none() {
            let url = format!("{}{}/Users/{}/Views", self.host, prefix, url_encode(user_id));
            let response = self
                .client
                .get(&url)
                .headers(self.build_headers()?)
                .send()
                .await?;

            let response = check_response(response)?;
            let views: ItemsResponse = json_with_limit(response).await?;
            return Ok(FsListResponse {
                items: views.items,
                paths: vec![PathInfo {
                    name: "Home".to_string(),
                    path: String::new(),
                }],
                total: views.total_record_count,
            });
        }

        // Query items with filters
        let mut url = format!(
            "{}{}/Users/{}/Items?StartIndex={}&Limit={}",
            self.host, prefix, url_encode(user_id), start_index, limit
        );

        if let Some(p) = path {
            url.push_str(&format!("&ParentId={}", url_encode(p)));
        }

        if let Some(term) = search_term {
            url.push_str(&format!("&SearchTerm={}&Recursive=true", url_encode(term)));
        }

        let response = self
            .client
            .get(&url)
            .headers(self.build_headers()?)
            .send()
            .await?;

        let response = check_response(response)?;
        let items: ItemsResponse = json_with_limit(response).await?;

        let mut paths = vec![PathInfo {
            name: "Home".to_string(),
            path: String::new(),
        }];

        // Add current path if specified
        if let Some(p) = path {
            if let Ok(item) = self.get_item(p).await {
                paths.push(PathInfo {
                    name: item.name,
                    path: item.id,
                });
            }
        }

        Ok(FsListResponse {
            items: items.items,
            paths,
            total: items.total_record_count,
        })
    }

    /// Logout
    pub async fn logout(&self) -> Result<(), EmbyError> {
        let prefix = self.get_api_prefix();
        let url = format!("{}{}/Sessions/Logout", self.host, prefix);

        self.client
            .post(&url)
            .headers(self.build_headers()?)
            .send()
            .await?;

        Ok(())
    }

    /// Get playback information
    pub async fn get_playback_info(
        &self,
        item_id: &str,
        media_source_id: Option<&str>,
        audio_stream_index: Option<i32>,
        subtitle_stream_index: Option<i32>,
        max_streaming_bitrate: Option<i64>,
    ) -> Result<PlaybackInfoResponse, EmbyError> {
        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| EmbyError::InvalidConfig("Missing user_id".to_string()))?;

        let prefix = self.get_api_prefix();
        let url = format!("{}{}/Items/{}/PlaybackInfo", self.host, prefix, url_encode(item_id));

        let mut body = json!({
            "UserId": user_id,
            "DeviceProfile": default_device_profile(),
        });

        if let Some(source_id) = media_source_id {
            body["MediaSourceId"] = json!(source_id);
        }
        if let Some(audio_idx) = audio_stream_index {
            body["AudioStreamIndex"] = json!(audio_idx);
        }
        if let Some(sub_idx) = subtitle_stream_index {
            body["SubtitleStreamIndex"] = json!(sub_idx);
        }
        if let Some(bitrate) = max_streaming_bitrate {
            body["MaxStreamingBitrate"] = json!(bitrate);
        }

        let response = self
            .client
            .post(&url)
            .headers(self.build_headers()?)
            .json(&body)
            .send()
            .await?;

        let response = check_response(response)?;
        let playback_info: PlaybackInfoResponse = json_with_limit(response).await?;
        Ok(playback_info)
    }

    /// Delete active encodings
    pub async fn delete_active_encodings(&self, play_session_id: &str) -> Result<(), EmbyError> {
        let prefix = self.get_api_prefix();
        let url = format!(
            "{}{}/Videos/ActiveEncodings?PlaySessionId={}",
            self.host, prefix, url_encode(play_session_id)
        );

        self.client
            .delete(&url)
            .headers(self.build_headers()?)
            .send()
            .await?;

        Ok(())
    }

    /// Get host URL
    #[must_use] 
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Check if client has credentials
    #[must_use] 
    pub const fn has_credentials(&self) -> bool {
        self.token.is_some() && self.user_id.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = EmbyClient::new("https://emby.example.com").unwrap();
        assert_eq!(client.host(), "https://emby.example.com");
        assert!(!client.has_credentials());

        let client_with_creds = EmbyClient::with_credentials(
            "https://emby.example.com",
            "test_token",
            "user123"
        ).unwrap();
        assert!(client_with_creds.has_credentials());
    }

    #[test]
    fn test_api_prefix_detection() {
        let emby_client = EmbyClient::new("https://emby.example.com").unwrap();
        assert_eq!(emby_client.get_api_prefix(), "/emby");

        let jellyfin_client = EmbyClient::new("https://jellyfin.example.com").unwrap();
        assert_eq!(jellyfin_client.get_api_prefix(), "/jellyfin");
    }
}
