//! Generic OIDC provider

use crate::oauth2::{Provider, OAuth2UserInfo};
use crate::Error;
use async_trait::async_trait;
use oauth2::{
    basic::BasicClient,
    AuthUrl, ClientId, ClientSecret, RedirectUrl, TokenUrl, TokenResponse,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// OIDC provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_url: String,
    #[serde(default)]
    pub issuer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub userinfo_url: Option<String>,
}

/// Generic OIDC provider
pub struct OidcProvider {
    client: Arc<BasicClient>,
    userinfo_url: Option<String>,
    http_client: Arc<Client>,
}

impl OidcProvider {
    /// Create a new OIDC provider with issuer (uses .well-known discovery)
    pub fn create(
        client_id: String,
        client_secret: String,
        redirect_url: String,
        issuer: &str,
    ) -> Self {
        let issuer = issuer.trim_end_matches('/');
        let client = Arc::new(
            BasicClient::new(
                ClientId::new(client_id),
                Some(ClientSecret::new(client_secret)),
                AuthUrl::new(format!("{}/authorize", issuer)).unwrap(),
                Some(TokenUrl::new(format!("{}/token", issuer)).unwrap()),
            )
            .set_redirect_uri(RedirectUrl::new(redirect_url).unwrap()),
        );

        Self {
            client,
            userinfo_url: Some(format!("{}/userinfo", issuer)),
            http_client: Arc::new(Client::new()),
        }
    }

    /// Create a new OIDC provider with custom endpoints
    pub fn create_with_endpoints(
        client_id: String,
        client_secret: String,
        redirect_url: String,
        issuer: &str,
        auth_url: Option<String>,
        token_url: Option<String>,
        userinfo_url: Option<String>,
    ) -> Self {
        let client = Arc::new(
            BasicClient::new(
                ClientId::new(client_id),
                Some(ClientSecret::new(client_secret)),
                AuthUrl::new(
                    auth_url
                        .unwrap_or_else(|| format!("{}/authorize", issuer.trim_end_matches('/'))),
                )
                .unwrap(),
                Some(
                    TokenUrl::new(
                        token_url
                            .unwrap_or_else(|| format!("{}/token", issuer.trim_end_matches('/'))),
                    )
                    .unwrap(),
                ),
            )
            .set_redirect_uri(RedirectUrl::new(redirect_url).unwrap()),
        );

        Self {
            client,
            userinfo_url,
            http_client: Arc::new(Client::new()),
        }
    }
}

#[async_trait]
impl Provider for OidcProvider {
    fn provider_type(&self) -> &str {
        "oidc"
    }

    async fn new_auth_url(&self, state: &str) -> Result<String, Error> {
        let (auth_url, _csrf_token) = self
            .client
            .authorize_url(|| oauth2::CsrfToken::new(state.to_string()))
            .url();
        Ok(auth_url.to_string())
    }

    async fn get_user_info(&self, code: &str) -> Result<OAuth2UserInfo, Error> {
        // Exchange code for token
        let token = self
            .client
            .exchange_code(oauth2::AuthorizationCode::new(code.to_string()))
            .request_async(oauth2::reqwest::async_http_client)
            .await
            .map_err(|e| Error::Internal(format!("Failed to exchange code: {}", e)))?;

        // Fetch user info from userinfo endpoint
        let userinfo_url = self
            .userinfo_url
            .as_ref()
            .ok_or_else(|| Error::Internal("userinfo_url not configured".to_string()))?;

        let resp = self
            .http_client
            .get(userinfo_url)
            .header("Authorization", format!("Bearer {}", token.access_token().secret()))
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to fetch user info: {}", e)))?
            .error_for_status()
            .map_err(|e| Error::Internal(format!("OIDC API error: {}", e)))?;

        #[derive(Deserialize)]
        struct OidcUser {
            sub: String,
            name: Option<String>,
            email: Option<String>,
            picture: Option<String>,
        }

        let user: OidcUser = resp
            .json()
            .await
            .map_err(|e| Error::Internal(format!("Failed to parse user info: {}", e)))?;

        Ok(OAuth2UserInfo {
            provider_user_id: user.sub,
            username: user.name.unwrap_or_default(),
            email: user.email,
            avatar: user.picture,
        })
    }
}

/// Factory function for OIDC provider
pub fn oidc_factory(config: &serde_yaml::Value) -> Result<Box<dyn Provider>, Error> {
    let config: OidcConfig = serde_yaml::from_value(config.clone())
        .map_err(|e| Error::InvalidInput(format!("Invalid OIDC config: {}", e)))?;

    // Use create_with_endpoints if any custom endpoint is specified
    let provider = if config.auth_url.is_some()
        || config.token_url.is_some()
        || config.userinfo_url.is_some()
    {
        OidcProvider::create_with_endpoints(
            config.client_id,
            config.client_secret,
            config.redirect_url,
            &config.issuer,
            config.auth_url,
            config.token_url,
            config.userinfo_url,
        )
    } else {
        OidcProvider::create(
            config.client_id,
            config.client_secret,
            config.redirect_url,
            &config.issuer,
        )
    };

    Ok(Box::new(provider))
}
