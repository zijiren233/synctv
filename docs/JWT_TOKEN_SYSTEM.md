# JWT Token System Documentation / JWT令牌系统文档

## 概述 (Overview)

SyncTV 使用双令牌系统（Access Token 和 Refresh Token）来实现安全的用户认证。

SyncTV uses a dual-token system (Access Token and Refresh Token) to implement secure user authentication.

## 为什么要区分 Access Token 和 Refresh Token？(Why Distinguish Between Access and Refresh Tokens?)

### 问题：为什么不只使用一种长期有效的 token？
### Question: Why not just use a single long-lived token?

如果只使用一种长期有效的 token，会面临以下安全风险：

If we only used a single long-lived token, we would face these security risks:

1. **令牌泄露风险 (Token Leakage Risk)**
   - 每次 API 请求都需要发送 token
   - Token is sent with every API request
   - Token 可能在传输过程中被截获（尤其是在不安全的网络环境中）
   - Token could be intercepted during transmission (especially on insecure networks)
   - 如果 token 有效期很长（比如 30 天），一旦被盗取，攻击者可以在整个有效期内冒充用户
   - If the token is long-lived (e.g., 30 days), once stolen, an attacker can impersonate the user for the entire validity period

2. **撤销困难 (Difficult to Revoke)**
   - 长期有效的 token 一旦签发，很难在到期前撤销
   - Long-lived tokens are difficult to revoke before expiration
   - 即使用户修改了密码或权限，旧 token 仍然有效
   - Even if a user changes their password or permissions, the old token remains valid

3. **XSS 攻击风险 (XSS Attack Risk)**
   - 长期 token 如果存储在浏览器中，容易受到 XSS 攻击
   - Long-lived tokens stored in the browser are vulnerable to XSS attacks
   - 攻击者获取 token 后可以长期使用
   - Attackers can use the token for extended periods after obtaining it

## 双令牌系统的解决方案 (Dual-Token System Solution)

### Access Token (访问令牌)

**特点 (Characteristics):**
- **短期有效**: 1 小时
- **Short-lived**: 1 hour
- **频繁使用**: 每次 API 请求都需要携带
- **Frequently used**: Required for every API request
- **存储位置**: 内存中（不持久化到 localStorage）
- **Storage**: In memory (not persisted to localStorage)

**作用 (Purpose):**
- 用于验证每次 API 请求的身份
- Used to authenticate every API request
- 包含用户 ID，服务器可以根据 ID 实时从数据库获取最新的权限信息
- Contains user ID, server can fetch latest permissions from database in real-time
- 短期有效降低了令牌被盗取后的风险窗口
- Short validity period reduces the risk window if token is stolen

### Refresh Token (刷新令牌)

**特点 (Characteristics):**
- **长期有效**: 30 天
- **Long-lived**: 30 days
- **使用频率低**: 只在 access token 过期时使用
- **Infrequently used**: Only used when access token expires
- **存储位置**: 可以安全存储在 httpOnly cookie 或安全存储中
- **Storage**: Can be safely stored in httpOnly cookies or secure storage

**作用 (Purpose):**
- 用于获取新的 access token
- Used to obtain new access tokens
- 避免用户频繁登录
- Avoids frequent user logins
- 使用频率低，减少了在网络传输中被截获的机会
- Low usage frequency reduces chances of being intercepted during transmission

## 工作流程 (Workflow)

### 1. 初始登录 (Initial Login)

```
用户登录 → 服务器验证 → 返回 access_token + refresh_token
User login → Server validates → Returns access_token + refresh_token
```

```rust
// synctv-core/src/service/user.rs
pub async fn login(&self, username: String, password: String)
    -> Result<(User, String, String)> {
    // Verify username and password
    let user = self.repository.get_by_username(&username).await?;
    verify_password(&password, &user.password_hash).await?;

    // Generate both tokens
    let access_token = self.jwt_service.sign_token(&user.id, TokenType::Access)?;   // 1 hour
    let refresh_token = self.jwt_service.sign_token(&user.id, TokenType::Refresh)?; // 30 days

    Ok((user, access_token, refresh_token))
}
```

### 2. 正常 API 请求 (Normal API Request)

```
客户端请求 (携带 access_token) → 服务器验证 → 返回数据
Client request (with access_token) → Server validates → Returns data
```

### 3. Access Token 过期时 (When Access Token Expires)

```
客户端请求失败 (401) → 使用 refresh_token 请求新令牌 → 获得新的 access_token + refresh_token → 重试原请求
Client request fails (401) → Use refresh_token to request new tokens → Get new access_token + refresh_token → Retry original request
```

```rust
// synctv-core/src/service/user.rs
pub async fn refresh_token(&self, refresh_token: String)
    -> Result<(String, String)> {
    // Verify refresh token
    let claims = self.jwt_service.verify_refresh_token(&refresh_token)?;

    // Verify user still exists and is valid
    let user_id = UserId::from_string(claims.sub);
    let user = self.repository.get_by_id(&user_id).await?;

    // Generate new token pair
    let new_access_token = self.jwt_service.sign_token(&user.id, TokenType::Access)?;
    let new_refresh_token = self.jwt_service.sign_token(&user.id, TokenType::Refresh)?;

    Ok((new_access_token, new_refresh_token))
}
```

## 令牌类型对比 (Token Type Comparison)

| 特性 (Feature) | Access Token | Refresh Token |
|---------------|--------------|---------------|
| 有效期 (Lifetime) | 1 小时 (1 hour) | 30 天 (30 days) |
| 使用频率 (Usage Frequency) | 高 (每次请求) / High (every request) | 低 (仅刷新时) / Low (only when refreshing) |
| 网络暴露 (Network Exposure) | 高 / High | 低 / Low |
| 泄露后的风险窗口 (Risk Window if Leaked) | 1 小时内 / Within 1 hour | 30 天内 / Within 30 days |
| 是否可撤销 (Revocable) | 否* / No* | 否* / No* |

\* JWT 令牌一旦签发就无法撤销，但可以通过黑名单机制实现撤销（见下文）

\* JWT tokens cannot be revoked once issued, but revocation can be implemented through a blacklist mechanism (see below)

## 安全优势 (Security Benefits)

### 1. 最小化风险窗口 (Minimize Risk Window)

如果 access token 被盗：
- 攻击者只能使用 1 小时
- 1 小时后 token 自动失效

If access token is stolen:
- Attacker can only use it for 1 hour
- Token automatically expires after 1 hour

如果 refresh token 被盗（可能性较小，因为使用频率低）：
- 可以通过黑名单机制立即撤销
- 用户可以通过修改密码使所有 token 失效

If refresh token is stolen (less likely due to low usage frequency):
- Can be immediately revoked through blacklist mechanism
- User can invalidate all tokens by changing password

### 2. 权限实时更新 (Real-time Permission Updates)

Token 中**不包含**角色和权限信息，只包含用户 ID：

Tokens do **not** contain role/permission information, only user ID:

```rust
// synctv-core/src/service/auth/jwt.rs
pub struct Claims {
    pub sub: String,      // User ID only
    pub typ: String,      // Token type
    pub iat: i64,         // Issued at
    pub exp: i64,         // Expiration
    // NOTE: No role/permissions field!
}
```

**为什么这样设计？(Why this design?)**

- 每次请求时从数据库实时获取用户的当前权限
- Fetch current user permissions from database in real-time with each request
- 即使 token 还在有效期内，权限变更也能立即生效
- Permission changes take effect immediately even if token is still valid
- 短期 access token (1小时) 配合实时权限查询，确保权限及时更新
- Short-lived access token (1 hour) combined with real-time permission queries ensures timely updates

### 3. 分离存储策略 (Separate Storage Strategies)

**推荐的客户端存储方式 (Recommended Client-side Storage):**

- **Access Token**: 存储在内存中（JavaScript 变量）
  - Storage in memory (JavaScript variable)
  - 页面刷新后重新用 refresh token 获取
  - Refresh after page reload using refresh token
  - 避免 XSS 攻击获取长期凭证
  - Prevents XSS attacks from obtaining long-term credentials

- **Refresh Token**: 存储在 httpOnly cookie 中
  - Store in httpOnly cookie
  - JavaScript 无法访问，防止 XSS
  - JavaScript cannot access, prevents XSS
  - 自动随请求发送到刷新端点
  - Automatically sent with requests to refresh endpoint

## Token 黑名单机制 (Token Blacklist Mechanism)

虽然 JWT 无法主动撤销，但可以通过黑名单实现撤销：

Although JWTs cannot be actively revoked, revocation can be implemented through a blacklist:

```rust
// synctv-core/src/service/token_blacklist.rs
pub struct TokenBlacklistService {
    // Uses Redis to store blacklisted tokens
    // Token expires from blacklist when its exp time is reached
}
```

**使用场景 (Use Cases):**
1. 用户登出时，将 refresh token 加入黑名单
   - Add refresh token to blacklist when user logs out
2. 用户修改密码时，将所有现有 token 加入黑名单
   - Add all existing tokens to blacklist when user changes password
3. 检测到异常活动时，立即撤销可疑 token
   - Immediately revoke suspicious tokens when abnormal activity is detected

## 最佳实践 (Best Practices)

### 服务器端 (Server-side)

1. **Access token 尽量短**
   - Keep access tokens short-lived
   - 当前设置：1 小时（可以根据安全需求调整为 15 分钟）
   - Current setting: 1 hour (can be adjusted to 15 minutes based on security requirements)

2. **Refresh token 适度长**
   - Keep refresh tokens moderately long
   - 当前设置：30 天（在便利性和安全性之间取得平衡）
   - Current setting: 30 days (balances convenience and security)

3. **每次刷新都返回新的 refresh token**
   - Return new refresh token on each refresh
   - 实现 token 轮换，增强安全性
   - Implements token rotation for enhanced security
   - 旧的 refresh token 可以加入黑名单
   - Old refresh tokens can be added to blacklist

4. **实时验证用户状态**
   - Validate user status in real-time
   - 即使 token 有效，也要检查用户是否被禁用、删除等
   - Even if token is valid, check if user is disabled, deleted, etc.

### 客户端 (Client-side)

1. **自动刷新机制**
   - Automatic refresh mechanism
   - 在 access token 即将过期前主动刷新（如剩余 5 分钟时）
   - Proactively refresh before access token expires (e.g., when 5 minutes remain)
   - 避免用户感知到令牌过期
   - Avoid user perception of token expiration

2. **安全存储**
   - Secure storage
   - Access token: 内存（或 sessionStorage）
   - Access token: memory (or sessionStorage)
   - Refresh token: httpOnly cookie
   - Refresh token: httpOnly cookie

3. **错误处理**
   - Error handling
   - 收到 401 响应时自动尝试刷新
   - Automatically attempt refresh on 401 response
   - 刷新失败后引导用户重新登录
   - Guide user to re-login after refresh failure

## 代码示例 (Code Examples)

### 客户端刷新逻辑 (Client-side Refresh Logic)

```javascript
// Example: Automatic token refresh
class AuthService {
  constructor() {
    this.accessToken = null;
    this.refreshToken = null;
    this.refreshPromise = null;
  }

  async request(url, options = {}) {
    // Add access token to request
    options.headers = {
      ...options.headers,
      'Authorization': `Bearer ${this.accessToken}`
    };

    let response = await fetch(url, options);

    // If 401, try to refresh token
    if (response.status === 401 && !options._retry) {
      const refreshed = await this.refreshAccessToken();
      if (refreshed) {
        // Retry with new token
        options._retry = true;
        options.headers['Authorization'] = `Bearer ${this.accessToken}`;
        response = await fetch(url, options);
      } else {
        // Refresh failed, redirect to login
        this.redirectToLogin();
      }
    }

    return response;
  }

  async refreshAccessToken() {
    // Prevent multiple simultaneous refresh requests
    if (this.refreshPromise) {
      return this.refreshPromise;
    }

    this.refreshPromise = fetch('/api/auth/refresh', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ refresh_token: this.refreshToken })
    })
    .then(res => res.json())
    .then(data => {
      this.accessToken = data.access_token;
      this.refreshToken = data.refresh_token;
      this.refreshPromise = null;
      return true;
    })
    .catch(err => {
      this.refreshPromise = null;
      return false;
    });

    return this.refreshPromise;
  }
}
```

## Guest Token (访客令牌)

除了用户的 access/refresh token，系统还支持访客 token：

In addition to user access/refresh tokens, the system also supports guest tokens:

```rust
pub enum TokenType {
    Access,   // 1 hour - for authenticated users
    Refresh,  // 30 days - for authenticated users
    Guest,    // 4 hours - for anonymous guests
}
```

**Guest Token 特点 (Guest Token Characteristics):**
- 有效期：4 小时
- Lifetime: 4 hours
- 无需数据库存储，完全无状态
- No database storage required, completely stateless
- 包含房间 ID 和会话 ID
- Contains room ID and session ID
- 用于临时访客访问特定房间
- Used for temporary guest access to specific rooms

详见：[GUEST_SYSTEM_DESIGN.md](./GUEST_SYSTEM_DESIGN.md)

For details, see: [GUEST_SYSTEM_DESIGN.md](./GUEST_SYSTEM_DESIGN.md)

## 总结 (Summary)

**为什么需要 Access Token 和 Refresh Token？**

**Why do we need both Access Token and Refresh Token?**

1. **安全性 (Security)**: 短期 access token 降低泄露风险，长期 refresh token 减少登录频率
   - Short-lived access tokens reduce leakage risk, long-lived refresh tokens reduce login frequency

2. **可用性 (Usability)**: 用户不需要频繁登录，access token 自动刷新
   - Users don't need to frequently log in, access tokens auto-refresh

3. **实时性 (Real-time)**: 短期 token + 实时权限查询，确保权限变更及时生效
   - Short-lived tokens + real-time permission queries ensure timely permission updates

4. **可撤销性 (Revocability)**: 通过黑名单机制和短期 token，可以快速撤销访问权限
   - Through blacklist mechanism and short-lived tokens, access can be quickly revoked

这是业界标准的 OAuth 2.0 / JWT 认证模式，在安全性和便利性之间取得了良好的平衡。

This is the industry-standard OAuth 2.0 / JWT authentication pattern that achieves a good balance between security and convenience.

## 相关文件 (Related Files)

- JWT 实现: `synctv-core/src/service/auth/jwt.rs`
- JWT Implementation: `synctv-core/src/service/auth/jwt.rs`
- 用户服务: `synctv-core/src/service/user.rs`
- User Service: `synctv-core/src/service/user.rs`
- Token 黑名单: `synctv-core/src/service/token_blacklist.rs`
- Token Blacklist: `synctv-core/src/service/token_blacklist.rs`
- API 定义: `synctv-proto/proto/client.proto`
- API Definition: `synctv-proto/proto/client.proto`
