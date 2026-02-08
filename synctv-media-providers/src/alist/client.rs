//! Alist HTTP Client
//!
//! Pure HTTP client for Alist API, no dependency on `MediaProvider`

use reqwest::{Client, header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, ORIGIN, REFERER, USER_AGENT}};
use serde_json::json;

use super::error::AlistError;
use super::types::{AlistResp, LoginData, HttpFsGetResp, HttpFsListResp, HttpFsOtherResp, HttpMeResp, HttpFsSearchResp};

/// Alist HTTP Client
///
/// Provides methods for interacting with Alist API:
/// - Authentication (login)
/// - File operations (fs/get, fs/list, fs/other)
pub struct AlistClient {
    host: String,
    token: Option<String>,
    client: Client,
}

impl AlistClient {
    /// Create a new Alist client
    pub fn new(host: impl Into<String>) -> Result<Self, AlistError> {
        Ok(Self {
            host: host.into(),
            token: None,
            client: Client::new(),
        })
    }

    /// Create a new Alist client with token
    pub fn with_token(host: impl Into<String>, token: impl Into<String>) -> Result<Self, AlistError> {
        Ok(Self {
            host: host.into(),
            token: Some(token.into()),
            client: Client::new(),
        })
    }

    /// Set authentication token
    pub fn set_token(&mut self, token: impl Into<String>) {
        self.token = Some(token.into());
    }

    /// Get current host
    #[must_use] 
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Check if client has token
    #[must_use] 
    pub const fn has_token(&self) -> bool {
        self.token.is_some()
    }

    /// Build request headers
    fn build_headers(&self) -> Result<HeaderMap, AlistError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(USER_AGENT, HeaderValue::from_static("Mozilla/5.0"));
        headers.insert(ORIGIN, HeaderValue::from_str(&self.host)?);
        headers.insert(REFERER, HeaderValue::from_str(&format!("{}/", self.host))?);

        if let Some(ref token) = self.token {
            headers.insert(AUTHORIZATION, HeaderValue::from_str(token)?);
        }

        Ok(headers)
    }

    /// Login to Alist server
    ///
    /// Returns authentication token on success
    pub async fn login(&mut self, username: &str, password: &str) -> Result<String, AlistError> {
        let url = format!("{}/api/auth/login", self.host);
        let body = json!({
            "username": username,
            "password": password,
        });

        let response = self
            .client
            .post(&url)
            .headers(self.build_headers()?)
            .json(&body)
            .send()
            .await?;

        let resp: AlistResp<LoginData> = response.json().await?;

        if resp.code != 200 {
            return Err(AlistError::Api {
                code: resp.code,
                message: resp.message,
            });
        }

        let token = resp.data.token;
        self.set_token(token.clone());
        Ok(token)
    }

    /// Get file/folder information
    ///
    /// # Arguments
    /// * `path` - File or directory path
    /// * `password` - Optional password for protected directories
    pub async fn fs_get(&self, path: &str, password: Option<&str>) -> Result<HttpFsGetResp, AlistError> {
        let url = format!("{}/api/fs/get", self.host);
        let body = json!({
            "path": path,
            "password": password.unwrap_or(""),
        });

        let response = self
            .client
            .post(&url)
            .headers(self.build_headers()?)
            .json(&body)
            .send()
            .await?;

        let resp: AlistResp<HttpFsGetResp> = response.json().await?;

        if resp.code != 200 {
            return Err(AlistError::Api {
                code: resp.code,
                message: resp.message,
            });
        }

        Ok(resp.data)
    }

    /// List directory contents
    ///
    /// # Arguments
    /// * `path` - Directory path
    /// * `page` - Page number (1-indexed)
    /// * `per_page` - Items per page
    /// * `password` - Optional password for protected directories
    pub async fn fs_list(
        &self,
        path: &str,
        page: u64,
        per_page: u64,
        password: Option<&str>,
    ) -> Result<HttpFsListResp, AlistError> {
        let url = format!("{}/api/fs/list", self.host);
        let body = json!({
            "path": path,
            "password": password.unwrap_or(""),
            "page": page,
            "per_page": per_page,
            "refresh": false,
        });

        let response = self
            .client
            .post(&url)
            .headers(self.build_headers()?)
            .json(&body)
            .send()
            .await?;

        let resp: AlistResp<HttpFsListResp> = response.json().await?;

        if resp.code != 200 {
            return Err(AlistError::Api {
                code: resp.code,
                message: resp.message,
            });
        }

        Ok(resp.data)
    }

    /// Get video preview information (for instances supporting transcoding)
    ///
    /// # Arguments
    /// * `path` - File path
    /// * `method` - Method name (e.g., "`video_preview`")
    /// * `password` - Optional password for protected directories
    pub async fn fs_other(
        &self,
        path: &str,
        method: &str,
        password: Option<&str>,
    ) -> Result<HttpFsOtherResp, AlistError> {
        let url = format!("{}/api/fs/other", self.host);
        let body = json!({
            "path": path,
            "method": method,
            "password": password.unwrap_or(""),
        });

        let response = self
            .client
            .post(&url)
            .headers(self.build_headers()?)
            .json(&body)
            .send()
            .await?;

        let resp: AlistResp<HttpFsOtherResp> = response.json().await?;

        if resp.code != 200 {
            return Err(AlistError::Api {
                code: resp.code,
                message: resp.message,
            });
        }

        Ok(resp.data)
    }

    /// Get current user information
    ///
    /// Requires authentication token
    pub async fn me(&self) -> Result<HttpMeResp, AlistError> {
        let url = format!("{}/api/me", self.host);

        let response = self
            .client
            .get(&url)
            .headers(self.build_headers()?)
            .send()
            .await?;

        let resp: AlistResp<HttpMeResp> = response.json().await?;

        if resp.code != 200 {
            return Err(AlistError::Api {
                code: resp.code,
                message: resp.message,
            });
        }

        Ok(resp.data)
    }

    /// Search files and directories
    ///
    /// # Arguments
    /// * `parent` - Parent directory path
    /// * `keywords` - Search keywords
    /// * `scope` - Search scope (0: current dir, 1: recursive)
    /// * `page` - Page number (1-indexed)
    /// * `per_page` - Items per page
    /// * `password` - Optional password for protected directories
    pub async fn fs_search(
        &self,
        parent: &str,
        keywords: &str,
        scope: u64,
        page: u64,
        per_page: u64,
        password: Option<&str>,
    ) -> Result<HttpFsSearchResp, AlistError> {
        let url = format!("{}/api/fs/search", self.host);
        let body = json!({
            "parent": parent,
            "keywords": keywords,
            "scope": scope,
            "page": page,
            "per_page": per_page,
            "password": password.unwrap_or(""),
        });

        let response = self
            .client
            .post(&url)
            .headers(self.build_headers()?)
            .json(&body)
            .send()
            .await?;

        let resp: AlistResp<HttpFsSearchResp> = response.json().await?;

        if resp.code != 200 {
            return Err(AlistError::Api {
                code: resp.code,
                message: resp.message,
            });
        }

        Ok(resp.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = AlistClient::new("https://alist.example.com").unwrap();
        assert_eq!(client.host(), "https://alist.example.com");
        assert!(!client.has_token());

        let client_with_token = AlistClient::with_token("https://alist.example.com", "test_token").unwrap();
        assert!(client_with_token.has_token());
    }

    #[test]
    fn test_set_token() {
        let mut client = AlistClient::new("https://alist.example.com").unwrap();
        assert!(!client.has_token());

        client.set_token("new_token");
        assert!(client.has_token());
    }
}
