# Proto RPC Naming Convention Analysis

## Overview
This document analyzes the naming conventions used in the gRPC service definitions (proto files) and identifies inconsistencies and areas for improvement.

## Current Naming Convention Issues

### 1. Inconsistent Use of "Set" vs "Update"

**Problem**: The codebase uses both `Set` and `Update` prefixes inconsistently, which creates confusion about semantics.

#### In client.proto:
- ✅ `SetUsername` - Makes sense, replaces the entire username
- ✅ `SetPassword` - Makes sense, replaces the entire password
- ❌ `SetRoomSettings` - Inconsistent with `UpdateRoomSetting` (singular)
- ❌ `UpdateRoomSetting` - Inconsistent naming (singular vs plural)
- ❌ `SetPlaylist` - Unclear semantics
- ❌ `SetPlaying` - Confusing name, should be more explicit
- ❌ `SetMemberPermission` - Should be `UpdateMemberPermission` for clarity

#### In admin.proto:
- ❌ `SetSettings` - Generic, should be more specific
- ❌ `SetProviderInstance` - Could be `UpdateProviderInstance`
- ❌ `SetUserPassword`, `SetUserUsername`, `SetUserRole` - Inconsistent with client API
- ❌ `SetRoomSettings` - Same as client.proto inconsistency
- ❌ `SetRoomPassword` - Should use Update for partial modifications

**Recommendation**:
- Use **`Set`** for complete replacement operations
- Use **`Update`** for partial modification operations
- Be consistent across all services

### 2. Action Verbs in RPC Names

**Problem**: Some RPC names contain action verbs that are not RESTful-style.

#### Examples:
- ❌ `NewPublishKey` - Should be `CreatePublishKey` for consistency
- ❌ `CheckRoom` - Should be `VerifyRoom` or `ValidateRoom`, or better yet, just use `GetRoom` with appropriate error handling
- ❌ `ConfirmEmail` - Could be `VerifyEmail` for clarity
- ❌ `ConfirmPasswordReset` - Could be `CompletePasswordReset`

**Recommendation**:
- Use standard CRUD verbs: `Create`, `Get`, `Update`, `Delete`, `List`
- For special operations, use clear descriptive verbs: `Verify`, `Validate`, `Approve`, `Ban`, `Unban`

### 3. Inconsistent Plurality in Method Names

**Problem**: Mix of singular and plural forms.

#### Examples:
- ✅ `GetRoomSettings` - Plural (settings is a collection)
- ❌ `SetRoomSettings` - Plural, but `UpdateRoomSetting` is singular
- ❌ `ResetRoomSettings` - Plural
- ✅ `ListPlaylists` - Correctly uses List prefix with plural
- ❌ `ListPlaylist` - Should be `GetPlaylist` (singular resource)

**Recommendation**:
- Use **plural** for operations on collections: `ListRooms`, `GetRoomSettings`
- Use **singular** for operations on single resources: `GetRoom`, `UpdateRoom`
- Prefix list operations with `List`, not `Get`

### 4. Media Service Naming Issues

**Problem**: Media service has confusing operation names.

#### Issues:
- ❌ `SetPlaylist` - Unclear what this does (replace? update? create?)
- ❌ `SetPlaying` - Confusing, should be `SetCurrentMedia` or `SelectMedia`
- ✅ `AddMedia`, `RemoveMedia` - Clear and consistent
- ✅ `SwapMedia` - Clear operation
- ✅ `Play`, `Pause`, `Seek` - Simple and clear playback controls
- ❌ `ChangeSpeed` - Should be `SetPlaybackSpeed` for clarity
- ❌ `SwitchMedia` - Redundant with `SetPlaying`

**Recommendation**:
- Rename `SetPlaylist` to `UpdatePlaylist` or `ReplacePlaylist` (be specific)
- Rename `SetPlaying` to `SetCurrentMedia`
- Rename `ChangeSpeed` to `SetPlaybackSpeed`
- Remove or merge `SwitchMedia` with `SetPlaying`/`SetCurrentMedia`

### 5. Room Settings Operations

**Problem**: Multiple overlapping methods for room settings.

#### Current Methods:
- `GetRoomSettings` - Get all settings
- `SetRoomSettings` - Set all settings (replace)
- `UpdateRoomSetting` - Update a single setting (partial)
- `ResetRoomSettings` - Reset to defaults

**Issues**:
- ❌ `UpdateRoomSetting` is singular but operates on a single field
- ❌ Inconsistent with HTTP PATCH semantics
- ❌ Not clear from name alone what each does

**Recommendation**:
```protobuf
rpc GetRoomSettings(GetRoomSettingsRequest) returns (GetRoomSettingsResponse);
rpc UpdateRoomSettings(UpdateRoomSettingsRequest) returns (UpdateRoomSettingsResponse);  // Partial update (PATCH semantics)
rpc ReplaceRoomSettings(ReplaceRoomSettingsRequest) returns (ReplaceRoomSettingsResponse);  // Full replacement (PUT semantics)
rpc ResetRoomSettings(ResetRoomSettingsRequest) returns (ResetRoomSettingsResponse);
```

### 6. Member Management Naming

**Problem**: Some operations are ambiguous.

#### Current:
- ✅ `KickMember` - Clear
- ✅ `BanMember`, `UnbanMember` - Clear pairs
- ❌ `SetMemberPermission` - Should be `UpdateMemberPermission`

**Recommendation**:
- Rename `SetMemberPermission` to `UpdateMemberPermissions` (plural, since it can update multiple permission bits)

### 7. User Management in Admin API

**Problem**: Admin API user management methods are verbose and inconsistent with client API.

#### Current Admin API:
- `SetUserPassword`, `SetUserUsername`, `SetUserRole` - Separate methods
- `BanUser`, `UnbanUser` - Status management

#### Client API:
- `SetUsername`, `SetPassword` - User manages their own

**Recommendation**:
- Keep separate methods for admin API (fine-grained control)
- But consider renaming to `UpdateUserPassword`, `UpdateUserUsername` for consistency
- Or create a unified `UpdateUser` method that can update multiple fields

### 8. Provider Management Naming

**Current:**
- ✅ `ListProviderInstances`
- ✅ `AddProviderInstance`
- ❌ `SetProviderInstance` - Should be `UpdateProviderInstance`
- ✅ `DeleteProviderInstance`
- ✅ `EnableProviderInstance`, `DisableProviderInstance`
- ❌ `ReconnectProviderInstance` - Could be shortened to `ReconnectProvider`

**Recommendation**:
- Rename `SetProviderInstance` to `UpdateProviderInstance`
- Keep Enable/Disable as separate operations (clear semantics)

## Proposed Standardized Naming Convention

### Standard Verbs by Operation Type:

1. **CRUD Operations**:
   - `Create{Resource}` - Create new resource
   - `Get{Resource}` - Get single resource by ID
   - `List{Resources}` - Get collection of resources (plural)
   - `Update{Resource}` - Partial update (PATCH semantics)
   - `Replace{Resource}` - Full replacement (PUT semantics) - optional, only if needed
   - `Delete{Resource}` - Delete resource

2. **State Management**:
   - `Enable{Resource}`, `Disable{Resource}` - Enable/disable functionality
   - `Activate{Resource}`, `Deactivate{Resource}` - Activate/deactivate state
   - `Ban{Resource}`, `Unban{Resource}` - Moderation actions
   - `Approve{Resource}`, `Reject{Resource}` - Approval workflows

3. **Membership Operations**:
   - `Join{Resource}` - Join as member
   - `Leave{Resource}` - Leave as member
   - `Kick{Member}` - Remove member (by moderator)
   - `Invite{Member}` - Invite member

4. **Playback Controls** (Media-specific):
   - `Play`, `Pause`, `Seek` - Simple verbs for media control
   - `SetPlaybackSpeed` - Explicit configuration
   - `SetCurrentMedia` - Select media to play

5. **Verification/Validation**:
   - `Verify{Resource}` - Verify validity
   - `Validate{Resource}` - Validate input/state
   - `Check{Resource}` - Quick status check

6. **Lifecycle**:
   - `Start{Resource}`, `Stop{Resource}` - Start/stop services
   - `Connect{Resource}`, `Disconnect{Resource}` - Connection management
   - `Reset{Resource}` - Reset to default state

## Priority Fixes

### High Priority (Breaking Changes - Do Now)

1. **Room Settings Consistency**:
   ```protobuf
   // Old
   rpc SetRoomSettings(SetRoomSettingsRequest) returns (SetRoomSettingsResponse);
   rpc UpdateRoomSetting(UpdateRoomSettingRequest) returns (UpdateRoomSettingResponse);

   // New
   rpc UpdateRoomSettings(UpdateRoomSettingsRequest) returns (UpdateRoomSettingsResponse);  // Unified PATCH
   rpc ReplaceRoomSettings(ReplaceRoomSettingsRequest) returns (ReplaceRoomSettingsResponse);  // Optional PUT
   ```

2. **Media Service Clarity**:
   ```protobuf
   // Old
   rpc SetPlaying(SetPlayingRequest) returns (SetPlayingResponse);
   rpc ChangeSpeed(ChangeSpeedRequest) returns (ChangeSpeedResponse);
   rpc SwitchMedia(SwitchMediaRequest) returns (SwitchMediaResponse);

   // New
   rpc SetCurrentMedia(SetCurrentMediaRequest) returns (SetCurrentMediaResponse);
   rpc SetPlaybackSpeed(SetPlaybackSpeedRequest) returns (SetPlaybackSpeedResponse);
   // Remove SwitchMedia (redundant with SetCurrentMedia)
   ```

3. **Member Permission Update**:
   ```protobuf
   // Old
   rpc SetMemberPermission(SetMemberPermissionRequest) returns (SetMemberPermissionResponse);

   // New
   rpc UpdateMemberPermissions(UpdateMemberPermissionsRequest) returns (UpdateMemberPermissionsResponse);
   ```

4. **Provider Management**:
   ```protobuf
   // Old
   rpc SetProviderInstance(SetProviderInstanceRequest) returns (SetProviderInstanceResponse);

   // New
   rpc UpdateProviderInstance(UpdateProviderInstanceRequest) returns (UpdateProviderInstanceResponse);
   ```

5. **Publish Key Creation**:
   ```protobuf
   // Old
   rpc NewPublishKey(NewPublishKeyRequest) returns (NewPublishKeyResponse);

   // New
   rpc CreatePublishKey(CreatePublishKeyRequest) returns (CreatePublishKeyResponse);
   ```

### Medium Priority (Nice to Have)

1. **CheckRoom Clarification**:
   ```protobuf
   // Old
   rpc CheckRoom(CheckRoomRequest) returns (CheckRoomResponse);

   // New - Could just use GetRoom and handle errors appropriately
   rpc VerifyRoomAccess(VerifyRoomAccessRequest) returns (VerifyRoomAccessResponse);
   ```

2. **Email Confirmation**:
   ```protobuf
   // Old
   rpc ConfirmEmail(ConfirmEmailRequest) returns (ConfirmEmailResponse);
   rpc ConfirmPasswordReset(ConfirmPasswordResetRequest) returns (ConfirmPasswordResetResponse);

   // New
   rpc VerifyEmail(VerifyEmailRequest) returns (VerifyEmailResponse);
   rpc CompletePasswordReset(CompletePasswordResetRequest) returns (CompletePasswordResetResponse);
   ```

### Low Priority (Future Consideration)

1. **Admin User Management Consistency**:
   - Consider unified `UpdateUser` instead of separate `SetUserPassword`, `SetUserUsername`, `SetUserRole`
   - Keep both for backward compatibility and fine-grained access control

2. **Settings Management**:
   ```protobuf
   // Old
   rpc SetSettings(SetSettingsRequest) returns (SetSettingsResponse);

   // New
   rpc UpdateSettings(UpdateSettingsRequest) returns (UpdateSettingsResponse);
   ```

## Implementation Strategy

Since the user requested no backward compatibility:

1. **Update proto definitions** with new standardized names
2. **Update ClientApiImpl and AdminApiImpl** to match new method names
3. **Update HTTP handlers** to call new method names
4. **Regenerate proto code**: `cargo build` will regenerate Rust code from .proto files
5. **Test all endpoints** to ensure functionality is preserved

## Summary Statistics

- **Total RPC methods analyzed**: ~60 methods across client.proto and admin.proto
- **High priority naming issues**: 9 methods need immediate renaming
- **Medium priority issues**: 3 methods could be improved
- **Low priority considerations**: 5+ methods for future standardization

## Conclusion

The proto RPC naming conventions show several inconsistencies that reduce API clarity and maintainability. The most critical issues are:

1. Inconsistent use of `Set` vs `Update`
2. Confusing media playback operation names
3. Singular vs plural inconsistencies
4. Non-standard verb usage

Implementing the high-priority fixes will significantly improve API consistency and align with RESTful and gRPC best practices.
