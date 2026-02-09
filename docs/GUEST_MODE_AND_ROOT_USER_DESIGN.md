# Guest Mode and Root User Initialization - Architecture Design

## Overview

This document outlines the design for improving guest mode support and root user initialization in SyncTV.

## Requirements Analysis

Based on the problem statement, we need to implement:

1. **Root User Initialization**: Auto-create root user on first startup with configurable default password
2. **Guest Mode Support**:
   - Global guest mode toggle
   - Room-level guest mode toggle
   - Guest authentication without database storage (no special guest user in DB)
   - Password-protected rooms block guest access

## Architecture Design

### 1. Root User Initialization

#### Configuration
Add new configuration section in `synctv-core/src/config.rs`:

```rust
pub struct BootstrapConfig {
    /// Whether to create root user on first startup
    pub create_root_user: bool,
    /// Root username (default: "root")
    pub root_username: String,
    /// Root password (default: "root" - should be changed!)
    pub root_password: String,
}
```

Environment variables:
- `SYNCTV_BOOTSTRAP_CREATE_ROOT_USER` (default: true)
- `SYNCTV_BOOTSTRAP_ROOT_USERNAME` (default: "root")
- `SYNCTV_BOOTSTRAP_ROOT_PASSWORD` (default: "root")

#### Implementation
Create bootstrap function in `synctv-core/src/bootstrap/user.rs`:

```rust
pub async fn bootstrap_root_user(
    pool: &PgPool,
    config: &BootstrapConfig
) -> Result<(), Error>
```

Logic:
1. Check if any user with `UserRole::Root` exists
2. If not, create root user with configured username/password
3. Set `role: UserRole::Root` and `status: UserStatus::Active`
4. Log creation (username and reminder to change password)

Call from `synctv/src/main.rs` after migrations, before service initialization.

### 2. Guest Mode Implementation

#### 2.1 Guest Authentication (Stateless)

**Key Design Decision**: Guests are NOT stored in database. Instead:

1. **Guest Token Generation**:
   - Use JWT with special `guest:` prefix in subject
   - Subject format: `guest:{room_id}:{random_session_id}`
   - Role: Special handling for guest tokens (not a UserRole)
   - No refresh token for guests (access token only)
   - Shorter TTL (e.g., 4 hours instead of 1 hour)

2. **Guest Token Claims**:
```rust
pub struct GuestTokenClaims {
    pub sub: String,           // "guest:{room_id}:{session_id}"
    pub room_id: String,
    pub session_id: String,    // Random ID for this guest session
    pub exp: i64,
    pub iat: i64,
}
```

3. **JWT Service Enhancement**:
Add methods to `JwtService`:
```rust
pub fn sign_guest_token(&self, room_id: &RoomId) -> Result<String>
pub fn verify_guest_token(&self, token: &str) -> Result<GuestTokenClaims>
pub fn is_guest_token(&self, token: &str) -> bool
```

#### 2.2 Room Settings Enhancement

The `RoomSettings` struct already has:
- `allow_guest_join: bool` ✓ (already exists)

We need to ensure this is properly enforced alongside:
- `require_password: bool` ✓ (already exists)

**Access Rule**: If room has password (`require_password: true`), guests cannot join UNLESS they provide the password and get upgraded to a member.

#### 2.3 Global Settings

The global setting `enable_guest: bool` already exists in `SettingsRegistry`. We need to ensure it's enforced globally.

#### 2.4 Guest Authentication Flow

**Endpoint**: `POST /api/room/{room_id}/guest/join`

**Request**:
```json
{
  "password": "optional_room_password"
}
```

**Logic**:
1. Check global `enable_guest` setting → return error if disabled
2. Get room settings → check `allow_guest_join` → return error if disabled
3. If room has password:
   - If password provided → verify it
   - If password correct → create regular room member (not guest) with Member role
   - If password missing/wrong → return error (guests blocked)
4. If no password required:
   - Generate guest token
   - Return guest token (access token only, no refresh)

**Response**:
```json
{
  "access_token": "eyJ...",
  "token_type": "guest",
  "expires_in": 14400,
  "room": { ... }
}
```

#### 2.5 Guest Permissions

Guests use the existing `guest_added_permissions` and `guest_removed_permissions` from `RoomSettings`.

Default guest permissions (from `PermissionBits::DEFAULT_GUEST`):
- VIEW_PLAYLIST ✓
- VIEW_CHAT_HISTORY ✓
- SEND_CHAT (maybe, configurable)
- NO admin/moderation permissions
- NO add/edit/delete media permissions

#### 2.6 Authentication Middleware Enhancement

Update HTTP authentication extractors in `synctv-api/src/http/auth.rs`:

```rust
pub enum AuthContext {
    User(User),
    Guest(GuestTokenClaims),
}

pub struct AuthUserOrGuest(pub AuthContext);
```

Logic:
1. Extract Bearer token
2. Check if token starts with "guest:" pattern → verify as guest token
3. Otherwise → verify as user token
4. Return appropriate AuthContext

#### 2.7 Room Join Validation

Update room join logic in `RoomService`:

```rust
pub async fn can_join_room(
    &self,
    room_id: &RoomId,
    auth: &AuthContext,
) -> Result<bool>
```

Logic:
1. If `AuthContext::User` → check user status/permissions (existing logic)
2. If `AuthContext::Guest`:
   - Check global `enable_guest`
   - Check room `allow_guest_join`
   - Check room `require_password` → if true, reject (guest must authenticate as member)
   - Return true/false

#### 2.8 WebSocket Connection Handling

Update WebSocket auth in `synctv-api/src/http/room.rs`:

1. Accept guest tokens in WebSocket upgrade
2. Track guest connections separately (for limits)
3. Guest sessions are ephemeral (no reconnect after disconnect)

### 3. API Changes

#### New Endpoints

1. **Guest Join**: `POST /api/room/{room_id}/guest/join`
   - Body: `{ "password": "optional" }`
   - Returns guest token or member token (if password provided)

2. **Guest Info**: `GET /api/guest/me`
   - Returns current guest session info
   - Only works with guest token

#### Modified Endpoints

1. **Room Join**: `POST /api/room/{room_id}/join`
   - Now checks guest restrictions
   - Returns different response for guest vs user

2. **Room Settings**: Update room settings to show `allow_guest_join` status

### 4. Database Schema

**No database changes needed!** This is a key feature - guests are stateless.

The existing schema already has:
- `room_settings.allow_guest_join` (JSON field) ✓
- `settings` table with `enable_guest` ✓
- Permission fields for guest customization ✓

### 5. Security Considerations

1. **Rate Limiting**: Guests should have stricter rate limits
2. **Session Management**: Guest tokens cannot be refreshed (must re-join)
3. **Connection Limits**: Separate limits for guest connections
4. **Audit Logging**: Guest actions should be logged with session ID
5. **Token Validation**: Guest tokens validated on every request (no user lookup)

### 6. Migration Strategy

Since the architecture already has partial guest support:

1. ✓ Global `enable_guest` setting exists
2. ✓ Room `allow_guest_join` setting exists
3. ✓ Guest permission customization exists
4. ✗ Guest authentication flow missing → **Implement**
5. ✗ Guest token generation missing → **Implement**
6. ✗ Root user bootstrap missing → **Implement**

### 7. Testing Strategy

1. **Unit Tests**:
   - Guest token generation/verification
   - Root user bootstrap logic
   - Guest permission checks

2. **Integration Tests**:
   - Guest join flow (with/without password)
   - Global guest toggle enforcement
   - Room guest toggle enforcement
   - Password protection blocking guests

3. **E2E Tests**:
   - Complete guest session lifecycle
   - WebSocket connection as guest
   - Guest rate limiting

## Implementation Plan

### Phase 1: Root User Bootstrap
1. Add `BootstrapConfig` to config.rs
2. Create `bootstrap/user.rs` with root user init
3. Call from main.rs after migrations
4. Add tests

### Phase 2: Guest Token System
1. Add guest token types to JWT service
2. Implement guest token generation/verification
3. Add `AuthUserOrGuest` extractor
4. Add tests

### Phase 3: Guest Authentication API
1. Create guest join endpoint
2. Update room join validation
3. Implement password verification for guests
4. Add tests

### Phase 4: WebSocket & Connection Management
1. Update WebSocket auth to accept guest tokens
2. Add guest connection tracking
3. Enforce connection limits
4. Add tests

### Phase 5: Documentation & E2E Tests
1. Update API documentation
2. Add E2E test suite
3. Update deployment guides

## Backward Compatibility

This design is fully backward compatible:
- Existing users unaffected
- Existing room settings preserved
- No database migration required
- Guest mode can be disabled globally

## References

- Old implementation (Go): Used special guest user in DB (ID: `00000000000000000000000000000001`)
- New implementation (Rust): Stateless guest tokens, no DB storage
- Similar pattern: OAuth2 stateless tokens vs session-based auth
