//! OAuth2/OIDC client model
//!
//! Stores OAuth2/OIDC authentication tokens for third-party login

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::UserId;

/// OAuth2/OIDC provider type
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OAuth2Provider {
    /// QQ
    QQ,
    /// GitHub
    GitHub,
    /// Google
    Google,
    /// Microsoft
    Microsoft,
    /// Discord
    Discord,
    /// Casdoor (OIDC)
    Casdoor,
    /// Logto (OIDC)
    Logto,
    /// Generic OIDC provider
    Oidc,
    /// Feishu SSO
    Feishu,
    /// Gitee
    Gitee,
}

impl OAuth2Provider {
    pub fn as_str(&self) -> &str {
        match self {
            Self::QQ => "qq",
            Self::GitHub => "github",
            Self::Google => "google",
            Self::Microsoft => "microsoft",
            Self::Discord => "discord",
            Self::Casdoor => "casdoor",
            Self::Logto => "logto",
            Self::Oidc => "oidc",
            Self::Feishu => "feishu",
            Self::Gitee => "gitee",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "qq" => Some(Self::QQ),
            "github" => Some(Self::GitHub),
            "google" => Some(Self::Google),
            "microsoft" => Some(Self::Microsoft),
            "discord" => Some(Self::Discord),
            "casdoor" => Some(Self::Casdoor),
            "logto" => Some(Self::Logto),
            "oidc" => Some(Self::Oidc),
            "feishu" => Some(Self::Feishu),
            "gitee" => Some(Self::Gitee),
            _ => None,
        }
    }

    /// Check if this provider type uses OIDC standard
    pub fn is_oidc(&self) -> bool {
        matches!(self, Self::Casdoor | Self::Logto | Self::Oidc | Self::Feishu | Self::Google | Self::Microsoft)
    }

    /// Get default scopes for this provider type
    pub fn default_scopes(&self) -> Vec<String> {
        if self.is_oidc() {
            vec!["openid".to_string(), "profile".to_string(), "email".to_string()]
        } else {
            vec!["identify".to_string()]
        }
    }
}

/// OAuth2/OIDC provider mapping (NO TOKENS)
///
/// Maps OAuth2 provider accounts to local users.
/// Tokens are NOT stored - only identity information for lookups.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserOAuthProviderMapping {
    pub id: String,
    pub provider: String,  // Stored as string in DB
    pub provider_user_id: String,
    pub user_id: UserId,
    pub username: String,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl UserOAuthProviderMapping {
    /// Get the provider as OAuth2Provider enum
    pub fn provider_enum(&self) -> Option<OAuth2Provider> {
        OAuth2Provider::from_str(&self.provider)
    }
}

/// OAuth2 user info from provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2UserInfo {
    pub provider: OAuth2Provider,
    pub provider_user_id: String,
    pub username: String,
    pub email: Option<String>,
    pub avatar: Option<String>,
}

/// OAuth2 authorization URL response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2AuthUrlResponse {
    pub url: String,
    pub state: String,
}

/// OAuth2 callback request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2CallbackRequest {
    pub code: String,
    pub state: String,
}

/// OAuth2 callback response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2CallbackResponse {
    pub token: Option<String>,  // JWT token if login
    pub redirect: Option<String>, // Redirect URL
}
