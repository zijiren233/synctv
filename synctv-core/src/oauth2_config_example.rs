//! OAuth2/OIDC 配置示例
//!
//! 此文件展示如何通过配置文件、环境变量配置多个OAuth2/OIDC提供商实例
//!
//! 配置优先级：环境变量 > 配置文件 > 默认值
//!
//! 使用方式：
//! 1. 将此配置保存为 config/oauth2.toml
//! 2. 或设置环境变量 (SYNCTV__OAUTH2__PROVIDERS__<INSTANCE_ID>__*)

// ============================================================
// 方式 1: TOML 配置文件示例 (config/oauth2.toml)
// ============================================================

/*
[oauth2]

# GitHub 实例 (使用默认端点)
[oauth2.github]
type = "github"
client_id = "your_github_client_id"
client_secret = "your_github_client_secret"

# Google 实例
[oauth2.google]
type = "google"
client_id = "your_google_client_id"
client_secret = "your_google_client_secret"

# Logto 实例 1 (自定义端点)
[oauth2.logto1]
type = "oidc"
issuer = "https://logto1.your-domain.com"
client_id = "logto1_client_id"
client_secret = "logto1_client_secret"
# scopes = ["openid", "profile", "email"]  # 可选，默认使用 provider type 的默认 scopes

# Logto 实例 2 (不同的 Logto 服务器)
[oauth2.logto2]
type = "oidc"
issuer = "https://logto2.your-domain.com"
client_id = "logto2_client_id"
client_secret = "logto2_client_secret"

# Casdoor 实例
[oauth2.casdoor_prod]
type = "casdoor"
endpoint = "https://casdoor.your-domain.com"
client_id = "casdoor_client_id"
client_secret = "casdoor_client_secret"

# QQ 实例
[oauth2.qq]
type = "qq"
client_id = "qq_client_id"
client_secret = "qq_client_secret"
app_id = "your_qq_app_id"

# 自定义 OIDC 提供商 (不支持 .well-known 的 OIDC 服务器)
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
// 方式 2: 环境变量配置示例
// ============================================================

/*
# 通用格式：SYNCTV__OAUTH2__<INSTANCE_ID>__<FIELD>

# GitHub 配置
SYNCTV__OAUTH2__GITHUB__TYPE=github
SYNCTV__OAUTH2__GITHUB__CLIENT_ID=xxx
SYNCTV__OAUTH2__GITHUB__CLIENT_SECRET=yyy

# Logto 实例 1
SYNCTV__OAUTH2__LOGTO1__TYPE=oidc
SYNCTV__OAUTH2__LOGTO1__ISSUER=https://logto1.your-domain.com
SYNCTV__OAUTH2__LOGTO1__CLIENT_ID=xxx
SYNCTV__OAUTH2__LOGTO1__CLIENT_SECRET=yyy

# Logto 实例 2
SYNCTV__OAUTH2__LOGTO2__TYPE=oidc
SYNCTV__OAUTH2__LOGTO2__ISSUER=https://logto2.your-domain.com
SYNCTV__OAUTH2__LOGTO2__CLIENT_ID=aaa
SYNCTV__OAUTH2__LOGTO2__CLIENT_SECRET=bbb

# Casdoor 配置
SYNCTV__OAUTH2__CASDOOR__TYPE=casdoor
SYNCTV__OAUTH2__CASDOOR__ENDPOINT=https://casdoor.your-domain.com
SYNCTV__OAUTH2__CASDOOR__CLIENT_ID=xxx
SYNCTV__OAUTH2__CASDOOR__CLIENT_SECRET=yyy

# QQ 配置
SYNCTV__OAUTH2__QQ__TYPE=qq
SYNCTV__OAUTH2__QQ__CLIENT_ID=xxx
SYNCTV__OAUTH2__QQ__CLIENT_SECRET=yyy
SYNCTV__OAUTH2__QQ__APP_ID=your_qq_app_id

# 自定义 OIDC 配置 (不支持 .well-known)
SYNCTV__OAUTH2__CUSTOM__TYPE=oidc
SYNCTV__OAUTH2__CUSTOM__ISSUER=https://custom.oidc.provider.com
SYNCTV__OAUTH2__CUSTOM__AUTH_URL=https://custom.oidc.provider.com/authorize
SYNCTV__OAUTH2__CUSTOM__TOKEN_URL=https://custom.oidc.provider.com/token
SYNCTV__OAUTH2__CUSTOM__USERINFO_URL=https://custom.oidc.provider.com/userinfo
SYNCTV__OAUTH2__CUSTOM__CLIENT_ID=xxx
SYNCTV__OAUTH2__CUSTOM__CLIENT_SECRET=yyy
*/

// ============================================================
// Provider Type 支持列表
// ============================================================

/*
支持的 provider type:
  - github    GitHub OAuth2
  - google    Google OAuth2 + OIDC
  - microsoft Microsoft OAuth2 + OIDC
  - discord   Discord OAuth2
  - qq        QQ OAuth2
  - casdoor   Casdoor OIDC
  - logto     Logto OIDC
  - feishu    飞书 SSO
  - gitee     Gitee OAuth2
  - oidc      通用 OIDC 提供商

OIDC Provider 配置:
  1. 如果支持 .well-known/openid-configuration (推荐):
     - 只需配置 issuer (会自动发现其他端点)
     - 示例: issuer = "https://accounts.google.com"

  2. 如果不支持 .well-known (或需要自定义):
     - 配置 auth_url, token_url, userinfo_url
     - 示例见下面的 "custom_oidc" 配置

默认 scopes:
  - OIDC providers: ["openid", "profile", "email"]
  - OAuth2 providers: ["identify"]
  - 可以通过 scopes 字段覆盖默认值
*/

// ============================================================
// 迁移指南：从旧的单一配置到多实例配置
// ============================================================

/*
旧配置 (不推荐):
  SYNCTV__OAUTH2__GITHUB__ENABLED=true
  SYNCTV__OAUTH2__GITHUB__CLIENT_ID=xxx
  SYNCTV__OAUTH2__GITHUB__CLIENT_SECRET=yyy

新配置 (推荐):
  [oauth2.github]
  type = "github"
  client_id = "xxx"
  client_secret = "yyy"

或使用环境变量:
  SYNCTV__OAUTH2__GITHUB__TYPE=github
  SYNCTV__OAUTH2__GITHUB__CLIENT_ID=xxx
  SYNCTV__OAUTH2__GITHUB__CLIENT_SECRET=yyy
*/

fn main() {
    println!("OAuth2 配置示例 - 请参考上述注释");
}
