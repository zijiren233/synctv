//! OAuth2/OIDC Configuration Examples
//!
//! This file demonstrates how to configure multiple OAuth2/OIDC provider instances
//! via configuration files and environment variables.
//!
//! Configuration priority: Environment variables > Config file > Defaults
//!
//! Usage:
//! 1. Save this configuration as config/oauth2.toml
//! 2. Or set environment variables (SYNCTV__OAUTH2__PROVIDERS__<INSTANCE_ID>__*)

// ============================================================
// Method 1: TOML Configuration File Example (config/oauth2.toml)
// ============================================================

/*
[oauth2]

# GitHub instance (using default endpoints)
[oauth2.github]
type = "github"
client_id = "your_github_client_id"
client_secret = "your_github_client_secret"

# Google instance
[oauth2.google]
type = "google"
client_id = "your_google_client_id"
client_secret = "your_google_client_secret"

# Logto instance 1 (custom endpoint)
[oauth2.logto1]
type = "oidc"
issuer = "https://logto1.your-domain.com"
client_id = "logto1_client_id"
client_secret = "logto1_client_secret"
# scopes = ["openid", "profile", "email"]  # Optional, defaults to provider type's default scopes

# Logto instance 2 (different Logto server)
[oauth2.logto2]
type = "oidc"
issuer = "https://logto2.your-domain.com"
client_id = "logto2_client_id"
client_secret = "logto2_client_secret"

# Casdoor instance
[oauth2.casdoor_prod]
type = "casdoor"
endpoint = "https://casdoor.your-domain.com"
client_id = "casdoor_client_id"
client_secret = "casdoor_client_secret"

# QQ instance
[oauth2.qq]
type = "qq"
client_id = "qq_client_id"
client_secret = "qq_client_secret"
app_id = "your_qq_app_id"

# Custom OIDC provider (OIDC server without .well-known support)
[oauth2.custom_oidc]
type = "oidc"
issuer = "https://custom.oidc.provider.com"
auth_url = "https://custom.oidc.provider.com/authorize"
token_url = "https://custom.oidc.provider.com/token"
userinfo_url = "https://custom.oidc.provider.com/userinfo"
client_id = "custom_client_id"
client_secret = "custom_client_secret"
*/

// ============================================================
// Method 2: Environment Variable Configuration Example
// ============================================================

/*
# General format: SYNCTV__OAUTH2__<INSTANCE_ID>__<FIELD>

# GitHub configuration
SYNCTV__OAUTH2__GITHUB__TYPE=github
SYNCTV__OAUTH2__GITHUB__CLIENT_ID=xxx
SYNCTV__OAUTH2__GITHUB__CLIENT_SECRET=yyy

# Logto instance 1
SYNCTV__OAUTH2__LOGTO1__TYPE=oidc
SYNCTV__OAUTH2__LOGTO1__ISSUER=https://logto1.your-domain.com
SYNCTV__OAUTH2__LOGTO1__CLIENT_ID=xxx
SYNCTV__OAUTH2__LOGTO1__CLIENT_SECRET=yyy

# Logto instance 2
SYNCTV__OAUTH2__LOGTO2__TYPE=oidc
SYNCTV__OAUTH2__LOGTO2__ISSUER=https://logto2.your-domain.com
SYNCTV__OAUTH2__LOGTO2__CLIENT_ID=aaa
SYNCTV__OAUTH2__LOGTO2__CLIENT_SECRET=bbb

# Casdoor configuration
SYNCTV__OAUTH2__CASDOOR__TYPE=casdoor
SYNCTV__OAUTH2__CASDOOR__ENDPOINT=https://casdoor.your-domain.com
SYNCTV__OAUTH2__CASDOOR__CLIENT_ID=xxx
SYNCTV__OAUTH2__CASDOOR__CLIENT_SECRET=yyy

# QQ configuration
SYNCTV__OAUTH2__QQ__TYPE=qq
SYNCTV__OAUTH2__QQ__CLIENT_ID=xxx
SYNCTV__OAUTH2__QQ__CLIENT_SECRET=yyy
SYNCTV__OAUTH2__QQ__APP_ID=your_qq_app_id

# Custom OIDC configuration (without .well-known support)
SYNCTV__OAUTH2__CUSTOM__TYPE=oidc
SYNCTV__OAUTH2__CUSTOM__ISSUER=https://custom.oidc.provider.com
SYNCTV__OAUTH2__CUSTOM__AUTH_URL=https://custom.oidc.provider.com/authorize
SYNCTV__OAUTH2__CUSTOM__TOKEN_URL=https://custom.oidc.provider.com/token
SYNCTV__OAUTH2__CUSTOM__USERINFO_URL=https://custom.oidc.provider.com/userinfo
SYNCTV__OAUTH2__CUSTOM__CLIENT_ID=xxx
SYNCTV__OAUTH2__CUSTOM__CLIENT_SECRET=yyy
*/

// ============================================================
// Supported Provider Types
// ============================================================

/*
Supported provider types:
  - github    GitHub OAuth2
  - google    Google OAuth2 + OIDC
  - microsoft Microsoft OAuth2 + OIDC
  - discord   Discord OAuth2
  - qq        QQ OAuth2
  - casdoor   Casdoor OIDC
  - logto     Logto OIDC
  - feishu    Feishu SSO
  - gitee     Gitee OAuth2
  - oidc      Generic OIDC provider

OIDC Provider configuration:
  1. If supports .well-known/openid-configuration (recommended):
     - Only configure issuer (will auto-discover other endpoints)
     - Example: issuer = "https://accounts.google.com"

  2. If does not support .well-known (or needs customization):
     - Configure auth_url, token_url, userinfo_url
     - See "custom_oidc" configuration below for example

Default scopes:
  - OIDC providers: ["openid", "profile", "email"]
  - OAuth2 providers: ["identify"]
  - Can override defaults via scopes field
*/

// ============================================================
// Migration Guide: From Old Single Config to Multi-Instance Config
// ============================================================

/*
Old configuration (not recommended):
  SYNCTV__OAUTH2__GITHUB__ENABLED=true
  SYNCTV__OAUTH2__GITHUB__CLIENT_ID=xxx
  SYNCTV__OAUTH2__GITHUB__CLIENT_SECRET=yyy

New configuration (recommended):
  [oauth2.github]
  type = "github"
  client_id = "xxx"
  client_secret = "yyy"

Or using environment variables:
  SYNCTV__OAUTH2__GITHUB__TYPE=github
  SYNCTV__OAUTH2__GITHUB__CLIENT_ID=xxx
  SYNCTV__OAUTH2__GITHUB__CLIENT_SECRET=yyy
*/

fn main() {
    println!("OAuth2 Configuration Examples - Please refer to the comments above");
}
