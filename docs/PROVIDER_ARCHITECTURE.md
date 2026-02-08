# Provider Architecture and Naming Conventions

## Overview

SyncTV uses a flexible provider system to support multiple media sources (Bilibili, Alist, Emby, Direct URLs). This document clarifies the architecture and naming conventions used throughout the codebase.

## Core Concepts

### Provider vs Source Provider

**Important**: The naming differs between internal (Rust/Database) and external (API/Proto) representations:

| Layer | Field Name | Example Value | Usage |
|-------|-----------|---------------|-------|
| **Proto/API** | `provider` | `"bilibili"` | External API contract |
| **Rust Model** | `source_provider` | `"bilibili"` | Internal Rust struct field |
| **Database** | `source_provider` | `"bilibili"` | PostgreSQL column name |

**Rationale**:
- The proto uses the shorter `provider` name for a cleaner API contract
- Internally, we use `source_provider` to distinguish from other provider-related concepts
- The conversion happens automatically in `media_to_proto()` function (synctv-api/src/impls/client.rs:1104)

### Provider Instance

A **provider instance** represents a specific configured instance of a provider type:

- **Provider Type**: The category (e.g., "bilibili", "alist", "emby")
- **Provider Instance Name**: Unique identifier for a configured instance (e.g., "bilibili_main", "alist_company")

Example:
```rust
// You can have multiple Bilibili provider instances:
// - "bilibili_cn"    → Bilibili CN datacenter
// - "bilibili_hk"    → Bilibili HK datacenter
// - "bilibili_backup" → Fallback instance
```

## Architecture Patterns

### Pattern 1: Direct Provider Usage (Legacy)

```rust
// Old pattern - still works but not recommended for new code
let provider = BilibiliProvider::new(config);
let playback = provider.generate_playback(media).await?;
```

**Use Case**: Unit tests, simple scenarios

### Pattern 2: Provider Registry (Recommended)

```rust
// New pattern - preferred for production code
let provider = providers_manager
    .get_provider("bilibili_main")
    .await?;
let playback = provider.generate_playback(media).await?;
```

**Use Case**: Production code, dynamic provider switching, multi-instance support

## Media Source Configuration

### Database Schema

```sql
CREATE TABLE media (
    id CHAR(12) PRIMARY KEY,
    source_provider VARCHAR(64) NOT NULL,      -- Provider type
    provider_instance_name VARCHAR(64),        -- Instance identifier
    source_config JSONB NOT NULL,              -- Provider-specific config
    -- ... other fields
);
```

### Rust Model

```rust
pub struct Media {
    pub source_provider: String,                    // e.g., "bilibili"
    pub provider_instance_name: Option<String>,     // e.g., "bilibili_main"
    pub source_config: JsonValue,                   // Provider-specific JSON
    // ... other fields
}
```

### Proto Definition

```protobuf
message Media {
    string provider = 4;                    // e.g., "bilibili"
    string provider_instance_name = 10;     // e.g., "bilibili_main"
    bytes source_config = 11;               // JSON bytes
}
```

## Provider-Specific Configuration

### source_config Field

The `source_config` field contains provider-specific JSON configuration:

**For Direct URLs:**
```json
{
  "url": "https://example.com/video.mp4",
  "quality": "1080p",
  "subtitles": [...]
}
```

**For Bilibili:**
```json
{
  "bvid": "BV1xx411c7XZ",
  "cid": 123456,
  "quality": 80
}
```

**For Alist:**
```json
{
  "path": "/videos/movie.mkv",
  "password": "encrypted_password"
}
```

**For Emby:**
```json
{
  "item_id": "abcd1234",
  "server_id": "server1"
}
```

**Important Design Principle**:
- The `source_config` should **ONLY** be parsed by the provider implementation itself
- The Media model and other code should treat it as opaque JSON
- This ensures provider implementations remain decoupled and extensible

## Provider Implementation

### Provider Trait

All providers implement the `Provider` trait:

```rust
#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn provider_type(&self) -> ProviderType;

    async fn generate_playback(
        &self,
        media: &Media,
    ) -> Result<PlaybackResult>;

    // Optional methods for dynamic providers
    async fn can_login(&self) -> bool { false }
    async fn list_content(&self, params: &ListParams) -> Result<Vec<ContentItem>> {
        Err(Error::NotSupported)
    }
}
```

### Provider Types

```rust
pub enum ProviderType {
    DirectUrl,  // Direct video URLs
    Bilibili,   // Bilibili video platform
    Alist,      // Alist file listing
    Emby,       // Emby media server
}
```

## Provider Instance Management

### Creating Provider Instances

```rust
// Admin creates a new provider instance
let instance = ProviderInstance {
    name: "bilibili_main".to_string(),
    providers: vec!["bilibili".to_string()],
    config: json!({
        "api_endpoint": "https://api.bilibili.com",
        "timeout_secs": 30
    }),
    enabled: true,
};

provider_instance_manager.add_instance(instance).await?;
```

### Using Provider Instances

```rust
// Application code looks up provider by instance name
let provider = providers_manager
    .get_provider("bilibili_main")
    .await?;

// Generate playback from media
let playback_result = provider.generate_playback(&media).await?;
```

### Dynamic Provider Switching

Users can switch which provider instance is used for a media item:

```rust
// Switch media to use different provider instance
media_service.update_provider_instance(
    media_id,
    "bilibili_backup"  // Switch to backup instance
).await?;
```

## Cluster Considerations

### Provider Instance Registry

In a clustered deployment:

1. **Provider instances are shared** across all nodes via the database
2. **Each node loads its own provider manager** from the shared configuration
3. **Provider instance changes are broadcast** via Redis pub/sub for hot-reload

### Cache Coherency

Provider-related caches need coordination:

```rust
// When provider instance is updated:
1. Update database record
2. Publish Redis event to all nodes
3. Each node invalidates its local cache
4. Next request loads fresh configuration
```

## Best Practices

### 1. Always Use Provider Instance Names

**DON'T:**
```rust
// Hard-coding provider types makes switching difficult
if media.source_provider == "bilibili" {
    // Use specific Bilibili API
}
```

**DO:**
```rust
// Look up provider via registry - supports dynamic switching
let provider = providers_manager
    .get_provider(&media.provider_instance_name?)
    .await?;
let playback = provider.generate_playback(&media).await?;
```

### 2. Validate Provider Instance Exists

```rust
// Before creating media, verify the provider instance exists
let provider = providers_manager
    .get_provider(&provider_instance_name)
    .await
    .map_err(|_| Error::ProviderInstanceNotFound)?;
```

### 3. Handle Provider Failures Gracefully

```rust
// Implement retry logic with fallback instances
let playback = try_with_fallback(
    vec!["bilibili_main", "bilibili_backup"],
    |instance_name| async move {
        let provider = providers_manager.get_provider(instance_name).await?;
        provider.generate_playback(&media).await
    }
).await?;
```

### 4. Document Provider-Specific Behavior

Each provider implementation should document:
- Required `source_config` fields
- Optional `source_config` fields
- Error conditions and retry semantics
- Rate limiting behavior
- Authentication requirements

## API Examples

### Creating Media with Provider Instance

**HTTP POST /api/rooms/:room_id/media**
```json
{
  "name": "Awesome Video",
  "provider": "bilibili",
  "provider_instance_name": "bilibili_main",
  "source_config": {
    "bvid": "BV1xx411c7XZ",
    "cid": 123456
  }
}
```

### Listing Available Provider Instances

**HTTP GET /api/admin/provider-instances**
```json
{
  "instances": [
    {
      "name": "bilibili_main",
      "providers": ["bilibili"],
      "enabled": true
    },
    {
      "name": "alist_company",
      "providers": ["alist"],
      "enabled": true
    }
  ]
}
```

## Related Files

- **Proto Definitions**: `synctv-proto/proto/client.proto`
- **Rust Model**: `synctv-core/src/models/media.rs`
- **Provider Trait**: `synctv-core/src/provider/mod.rs`
- **Provider Manager**: `synctv-core/src/service/providers_manager.rs`
- **Instance Management**: `synctv-core/src/repository/provider_instance.rs`
- **Conversion Logic**: `synctv-api/src/impls/client.rs:1084` (media_to_proto)
- **Database Schema**: `migrations/20240101000005_create_media_table.sql`

## Future Enhancements

### Planned Features

1. **Bulk Provider Operations**: Import multiple media items from a provider in one operation
2. **Provider Health Monitoring**: Track provider instance availability and latency
3. **Automatic Failover**: Switch to backup instances when primary fails
4. **Provider-Specific Rate Limiting**: Respect API quotas per provider
5. **Cross-Provider Search**: Search across multiple providers simultaneously
6. **Provider Analytics**: Track usage metrics per provider instance

### Extensibility

To add a new provider type:

1. Implement the `Provider` trait in a new module
2. Add the provider type to `ProviderType` enum
3. Register the provider in `ProvidersManager`
4. Update API documentation with provider-specific `source_config` schema
5. Add integration tests

## Summary

**Key Takeaways:**

1. `provider` (API) and `source_provider` (internal) refer to the same concept
2. Use provider instance names for dynamic provider management
3. Treat `source_config` as opaque JSON outside provider implementations
4. Prefer the registry pattern over direct provider instantiation
5. Document provider-specific configuration schemas
6. Handle provider failures with retry and fallback logic
