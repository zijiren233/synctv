# Guest System Design

## Overview

The guest system allows anonymous users to join rooms without creating an account. This document describes the complete guest system architecture, including authentication, permissions, and access control.

## Core Principles

1. **Stateless Authentication**: Guests use JWT tokens without database storage
2. **Real-time Permissions**: All permissions fetched from database on each request (not cached in JWT)
3. **Granular Control**: Both global and room-level guest mode toggles
4. **Security First**: Password-protected rooms automatically block guest access

## Guest Authentication

### Token Structure

Guest tokens are standard JWTs with the following claims:

```rust
pub struct GuestClaims {
    pub sub: String,           // Format: "guest:{room_id}:{session_id}"
    pub room_id: String,       // Room ID guest is joining
    pub session_id: String,    // Random 16-character session ID
    pub typ: String,           // Always "guest"
    pub iat: i64,             // Issued at timestamp
    pub exp: i64,             // Expiration timestamp (4 hours from issue)
}
```

### Key Features

- **No Database Storage**: Guests are NOT stored in the `users` or `room_members` tables
- **Session-Based**: Each guest token has a unique random session ID
- **Room-Scoped**: Guest token is only valid for the specific room ID it was issued for
- **Short-Lived**: Expires after 4 hours (no refresh tokens)
- **No Role Caching**: Role/permissions are NEVER stored in JWT - always fetched from DB

## Permission System

### Global Default Permissions

Defined in `SettingsRegistry`:

```rust
pub guest_default_permissions: Setting<u64>
```

Default value: `511` (0x1FF) = Read-only permissions:
- `VIEW_PLAYLIST` (1 << 40)
- `VIEW_MEMBER_LIST` (1 << 41)
- `VIEW_CHAT_HISTORY` (1 << 42)
- Basic viewing permissions

### Room-Level Overrides

Defined in `RoomSettings`:

```rust
pub guest_added_permissions: Option<u64>     // Extra permissions for guests
pub guest_removed_permissions: Option<u64>   // Denied permissions for guests
```

### Permission Calculation

Formula: `(global_default | room_added) & ~room_removed`

```rust
impl RoomSettings {
    pub fn guest_permissions(&self, global_default: PermissionBits) -> PermissionBits {
        Self::effective_permissions_for_role(
            global_default,
            self.guest_added_permissions,
            self.guest_removed_permissions,
        )
    }
}
```

Example:
```rust
// Global default: VIEW_PLAYLIST | VIEW_MEMBER_LIST (read-only)
let global_default = PermissionBits(511);

// Room adds: SEND_CHAT (allow guests to chat)
let room_added = Some(PermissionBits::SEND_CHAT);

// Room removes: VIEW_MEMBER_LIST (hide member list from guests)
let room_removed = Some(PermissionBits::VIEW_MEMBER_LIST);

// Effective: (global_default | room_added) & ~room_removed
// Result: VIEW_PLAYLIST | SEND_CHAT
```

## Access Control

### Three-Level Check

Guest access is validated at three levels:

#### 1. Global Toggle

```rust
pub enable_guest: Setting<bool>  // In SettingsRegistry
```

If disabled globally, NO guests can join ANY room.

#### 2. Room Toggle

```rust
pub allow_guest_join: bool  // In RoomSettings
```

Room owners can disable guest access to their specific room.

#### 3. Password Protection

```rust
pub require_password: RequirePassword  // In RoomSettings
```

**Automatic Rule**: If a room has a password, guests CANNOT join (even if both toggles are enabled).

**Reasoning**: Guests are ephemeral/anonymous. Password-protected rooms require authentication, which guests cannot provide.

### Validation Method

```rust
impl RoomService {
    pub async fn check_guest_allowed(
        &self,
        room_id: &RoomId,
        settings_registry: Option<&SettingsRegistry>,
    ) -> Result<()> {
        // 1. Check global enable_guest
        if let Some(registry) = settings_registry {
            if !registry.enable_guest.get().unwrap_or(true) {
                return Err(Error::Authorization("Guest mode disabled globally"));
            }
        }

        // 2. Check room allow_guest_join
        let room_settings = self.room_settings_repo.get(room_id).await?;
        if !room_settings.allow_guest_join.0 {
            return Err(Error::Authorization("Guest access not allowed in this room"));
        }

        // 3. Check room password
        if room_settings.require_password.0 {
            return Err(Error::Authorization(
                "Guests cannot join password-protected rooms"
            ));
        }

        Ok(())
    }
}
```

## Guest Lifecycle

### 1. Join Room

**Endpoint**: `POST /api/room/{room_id}/guest/join`

**Request**:
```json
{
  "password": "optional_if_room_has_password"
}
```

**Flow**:
```
1. Validate guest access (check_guest_allowed)
2. If password provided and correct → Create regular member (NOT guest)
3. If no password required → Generate guest token
4. Return token
```

**Response** (Guest):
```json
{
  "access_token": "eyJ...",
  "token_type": "guest",
  "expires_in": 14400,
  "room": { ... }
}
```

**Response** (Password → Member):
```json
{
  "access_token": "eyJ...",
  "token_type": "access",
  "expires_in": 3600,
  "member": { ... },
  "room": { ... }
}
```

### 2. Connect to Room (WebSocket)

**Authentication**:
- Guest token sent in `Authorization: Bearer {token}` header
- Server extracts and validates guest token
- Session tracked by `{room_id}:{session_id}`

**Connection Tracking**:
```rust
// In ConnectionManager
guest_connections: HashMap<(RoomId, String), Connection>  // session_id as key
```

### 3. Disconnect

**Triggers**:
- WebSocket close
- Token expiration (4 hours)
- Guest mode disabled (global or room level)
- Room password added
- Explicit kick by room admin

**Cleanup**:
- Remove from connection manager
- No database changes (guests not in DB)

## Kicking Guests on Mode Disable

### Event System

When guest mode is disabled, all guest connections must be terminated:

```rust
pub enum GuestKickReason {
    GlobalGuestModeDisabled,
    RoomGuestModeDisabled,
    RoomPasswordAdded,
    AdminKick,
}

impl NotificationService {
    pub async fn kick_all_guests(
        &self,
        room_id: &RoomId,
        reason: GuestKickReason,
    ) -> Result<()> {
        // Send notification to all guest connections in room
        let message = match reason {
            GuestKickReason::GlobalGuestModeDisabled =>
                "Guest mode has been disabled globally",
            GuestKickReason::RoomGuestModeDisabled =>
                "Guest access has been disabled for this room",
            GuestKickReason::RoomPasswordAdded =>
                "This room now requires authentication",
            GuestKickReason::AdminKick =>
                "You have been removed from the room",
        };

        self.notify_guests_kicked(room_id, message).await
    }
}
```

### Integration Points

**✅ Implemented:**

1. **Room Setting Change - `allow_guest_join`**:
```rust
// In RoomService::update_room_setting()
"allow_guest_join" => {
    if let Some(bool_val) = value.as_bool() {
        settings.allow_guest_join = AllowGuestJoin(bool_val);
        // If guest mode is disabled, kick all guests
        if !bool_val {
            notification_service.kick_all_guests(room_id,
                GuestKickReason::RoomGuestModeDisabled).await;
        }
    }
}
```

2. **Room Setting Change - `require_password`**:
```rust
// In RoomService::update_room_setting()
"require_password" => {
    if let Some(bool_val) = value.as_bool() {
        settings.require_password = RequirePassword(bool_val);
        // If password is now required, kick all guests
        if bool_val {
            notification_service.kick_all_guests(room_id,
                GuestKickReason::RoomPasswordAdded).await;
        }
    }
}
```

3. **Password Added via `update_room_password`**:
```rust
// In RoomService::update_room_password()
if let Some(pwd_hash) = password_hash {
    self.room_settings_repo.set(room_id, "password", &pwd_hash).await?;
    // Kick all guests when password is added
    notification_service.kick_all_guests(room_id,
        GuestKickReason::RoomPasswordAdded).await;
}
```

**⚠️ To Be Implemented:**

1. **Global Setting Change**:
```rust
// When SettingsRegistry.enable_guest is set to false
// This requires adding a settings change callback system
for room_id in all_rooms {
    notification_service.kick_all_guests(&room_id,
        GuestKickReason::GlobalGuestModeDisabled).await;
}
```

**Note**: Global guest mode disable requires a settings callback/observer system which is not yet implemented. For now, when the global `enable_guest` setting is disabled, new guests will be blocked from joining (enforced by `check_guest_allowed`), but existing guests will remain connected until their tokens expire or they disconnect naturally.

## Security Considerations

### Why Password-Protected Rooms Block Guests

1. **Authentication Requirement**: Password verification requires user identity
2. **Accountability**: Password-protected rooms imply content should be access-controlled
3. **Privacy**: Room owners expect password = authenticated users only
4. **Abuse Prevention**: Prevents guests from sharing password-protected content

### Token Security

1. **Short Expiration**: 4-hour lifetime prevents long-term impersonation
2. **No Refresh**: Forces periodic re-validation of guest access rules
3. **Room-Scoped**: Token only valid for one specific room
4. **Session Uniqueness**: Each join generates new session ID

### Permission Enforcement

1. **No JWT Role Storage**: Role/permissions never cached in token
2. **Real-Time Checks**: Every request fetches current permissions from DB
3. **Immediate Effect**: Permission changes apply instantly to all guests
4. **Fail-Safe**: If permission check fails, deny access

## Implementation Checklist

- [x] JWT Claims without role field
- [x] Guest token generation (JwtService::sign_guest_token)
- [x] Guest token verification (JwtService::verify_guest_token)
- [x] Guest access validation (RoomService::check_guest_allowed)
- [x] Global guest permissions (SettingsRegistry::guest_default_permissions)
- [x] Room guest permission overrides (RoomSettings::guest_added/removed_permissions)
- [x] GuestKickReason enum
- [x] RoomEvent::GuestKicked variant
- [x] NotificationService::kick_all_guests method
- [x] Room-level guest kick hooks (allow_guest_join, require_password)
- [ ] Global guest mode disable hook (requires settings callback system)
- [ ] Guest join API endpoint
- [ ] Guest WebSocket authentication
- [ ] Connection tracking for guests
- [ ] WebSocket handler for GuestKicked event

## Future Enhancements

1. **Guest Limits**: Maximum guests per room
2. **Guest Rate Limiting**: Stricter rate limits for guests
3. **Guest Analytics**: Track guest join/leave events
4. **Guest Invitations**: Pre-approved guest tokens
5. **Guest Nicknames**: Optional display names for guests
