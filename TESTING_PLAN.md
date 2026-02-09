# SyncTV Comprehensive Testing Plan

## Project Overview
SyncTV is a real-time synchronized video watching platform built in Rust, featuring:
- Multi-service architecture (Core, API, Stream, Cluster, SFU)
- PostgreSQL database with migrations
- Redis for caching and pub/sub
- gRPC and HTTP APIs
- RTMP/HLS/FLV streaming
- WebRTC for P2P/SFU video
- OAuth2 authentication
- Media provider integrations (Bilibili, Alist, Emby)

## Test Categories

### 1. Server Startup and Configuration Testing
**Goal**: Verify server starts correctly with various configuration options

#### 1.1 Minimal Configuration (Single Node)
- [ ] Start without Redis (single-node mode)
- [ ] Start without JWT keys (should fail or generate)
- [ ] Start with minimal database connection
- [ ] Verify graceful handling of missing optional services

#### 1.2 Full Configuration (Cluster Mode)
- [ ] Start with Redis enabled (cluster mode)
- [ ] Start with all WebRTC modes (signaling_only, peer_to_peer, hybrid, sfu)
- [ ] Start with streaming services (RTMP, HLS)
- [ ] Start with OAuth2 providers configured
- [ ] Start with STUN/TURN servers

#### 1.3 Configuration Validation
- [ ] Test invalid database URLs
- [ ] Test invalid Redis URLs
- [ ] Test invalid JWT key paths
- [ ] Test port conflicts
- [ ] Test invalid WebRTC configurations

### 2. Database Migration Testing
**Goal**: Ensure all migrations run successfully and data integrity

- [ ] Fresh database initialization
- [ ] Run all migrations from scratch
- [ ] Verify all tables created correctly
- [ ] Verify indexes and constraints
- [ ] Test rollback capability (if supported)
- [ ] Test migration idempotency
- [ ] Verify audit log partitions created

### 3. Authentication & Authorization Testing
**Goal**: Verify user authentication and authorization flows

#### 3.1 User Registration
- [ ] Register with valid credentials
- [ ] Register with duplicate username/email
- [ ] Register with invalid email format
- [ ] Register with weak password
- [ ] Verify password hashing (Argon2id)

#### 3.2 User Login
- [ ] Login with valid credentials
- [ ] Login with invalid password
- [ ] Login with non-existent user
- [ ] Verify JWT token generation
- [ ] Verify token contains correct claims

#### 3.3 Token Management
- [ ] Access token generation and validation
- [ ] Refresh token generation and validation
- [ ] Token expiration handling
- [ ] Token refresh flow
- [ ] Invalid token rejection
- [ ] Tampered token rejection

#### 3.4 Authorization
- [ ] User role-based access (User, Admin, Root)
- [ ] Room permission checks
- [ ] Media provider permission checks
- [ ] Verify permission bitmask operations

### 4. Room Management Testing
**Goal**: Test room lifecycle and operations

#### 4.1 Room Creation
- [ ] Create public room
- [ ] Create private room
- [ ] Create room with password
- [ ] Create room with description
- [ ] Create room with custom settings
- [ ] Verify creator gets owner permissions

#### 4.2 Room Joining
- [ ] Join public room
- [ ] Join private room (with permission)
- [ ] Join password-protected room (correct password)
- [ ] Join password-protected room (wrong password)
- [ ] Join non-existent room
- [ ] Join room at capacity

#### 4.3 Room Members
- [ ] List room members
- [ ] Update member permissions
- [ ] Kick member
- [ ] Ban member
- [ ] Unban member
- [ ] Leave room
- [ ] Verify permission inheritance

#### 4.4 Room Settings
- [ ] Get room settings
- [ ] Update room settings
- [ ] Reset room settings to defaults
- [ ] Verify settings validation

#### 4.5 Room Deletion
- [ ] Delete room as creator
- [ ] Delete room as non-creator (should fail)
- [ ] Verify cascade deletion of media/playlists
- [ ] Verify members removed

### 5. Media & Playlist Testing
**Goal**: Test media management and playlist operations

#### 5.1 Playlist Management
- [ ] Create static playlist
- [ ] Create dynamic playlist (with provider)
- [ ] Update playlist
- [ ] Delete playlist
- [ ] List playlists in room
- [ ] Verify root playlist exists

#### 5.2 Media Operations
- [ ] Add media to playlist
- [ ] Add media from URL
- [ ] Add media from provider (Bilibili/Alist/Emby)
- [ ] Remove media
- [ ] Swap media positions
- [ ] List media in playlist
- [ ] Verify media validation

#### 5.3 Playback Control
- [ ] Play media
- [ ] Pause media
- [ ] Seek to position
- [ ] Set playback speed
- [ ] Get playback state
- [ ] Set current media
- [ ] Verify sync across clients

### 6. Media Provider Testing
**Goal**: Test integration with external media providers

#### 6.1 Provider Instance Management
- [ ] Create provider instance (Alist, Bilibili, Emby)
- [ ] List provider instances
- [ ] Update provider instance
- [ ] Delete provider instance
- [ ] Test provider authentication

#### 6.2 Provider Operations
- [ ] Fetch media from Bilibili
- [ ] Fetch media from Alist
- [ ] Fetch media from Emby
- [ ] Verify media metadata parsing
- [ ] Handle provider errors gracefully

### 7. Streaming Testing (RTMP/HLS/FLV)
**Goal**: Test live streaming functionality

#### 7.1 Publish Key Management
- [ ] Generate publish key
- [ ] Validate publish key
- [ ] Verify publish key for stream
- [ ] Test key expiration
- [ ] Test invalid keys

#### 7.2 RTMP Publishing
- [ ] Publish stream to RTMP endpoint
- [ ] Verify stream authentication
- [ ] Test GOP cache functionality
- [ ] Test stream registration in Redis

#### 7.3 HLS Playback
- [ ] Request HLS playlist
- [ ] Download HLS segments
- [ ] Verify segment continuity
- [ ] Test stream state management

#### 7.4 FLV Playback
- [ ] Request FLV stream
- [ ] Verify FLV format
- [ ] Test GOP cache retrieval

### 8. WebRTC Testing
**Goal**: Test WebRTC signaling and media streaming

#### 8.1 ICE Server Configuration
- [ ] Get ICE servers (STUN)
- [ ] Get ICE servers (TURN)
- [ ] Verify TURN credential generation
- [ ] Test different WebRTC modes

#### 8.2 Signaling
- [ ] WebSocket connection establishment
- [ ] Offer/Answer exchange
- [ ] ICE candidate exchange
- [ ] Connection state management

#### 8.3 SFU Testing
- [ ] Trigger SFU mode (>= threshold participants)
- [ ] Verify simulcast layers
- [ ] Test peer limits
- [ ] Test room capacity

### 9. Real-Time Messaging Testing
**Goal**: Test chat and real-time events

#### 9.1 Chat Messages
- [ ] Send chat message
- [ ] Receive chat message
- [ ] Get chat history
- [ ] Verify message ordering
- [ ] Test message permissions

#### 9.2 Room Events
- [ ] User joined event
- [ ] User left event
- [ ] Playback state change event
- [ ] Media added event
- [ ] Settings changed event

### 10. Cluster Mode Testing
**Goal**: Test multi-node coordination

#### 10.1 Single Node Mode
- [ ] Verify works without Redis
- [ ] Verify local message routing
- [ ] Test connection limits (per-node)

#### 10.2 Cluster Mode
- [ ] Start multiple nodes with Redis
- [ ] Verify node ID generation
- [ ] Test message broadcasting
- [ ] Test message deduplication
- [ ] Verify connection tracking across nodes

### 11. OAuth2 Testing
**Goal**: Test OAuth2 authentication flows

- [ ] GitHub OAuth flow
- [ ] Google OAuth flow
- [ ] OIDC provider flow
- [ ] Logto provider flow
- [ ] Test callback handling
- [ ] Verify user linking
- [ ] Test token exchange

### 12. Admin API Testing
**Goal**: Test administrative operations

#### 12.1 User Management
- [ ] List all users
- [ ] Get user details
- [ ] Update user role
- [ ] Update user password
- [ ] Delete user
- [ ] Ban/unban user

#### 12.2 Room Management
- [ ] List all rooms
- [ ] Get room details
- [ ] Update room settings
- [ ] Delete any room
- [ ] Force close room

#### 12.3 System Settings
- [ ] Get system settings
- [ ] Update system settings
- [ ] Reset system settings

### 13. Performance & Load Testing
**Goal**: Verify system performance under load

- [ ] Concurrent user connections
- [ ] Concurrent room operations
- [ ] Large playlist operations
- [ ] High-frequency playback events
- [ ] Memory leak detection
- [ ] Connection pool exhaustion

### 14. Error Handling Testing
**Goal**: Verify graceful error handling

- [ ] Database connection loss
- [ ] Redis connection loss
- [ ] Network timeouts
- [ ] Invalid request payloads
- [ ] Rate limiting enforcement
- [ ] Circuit breaker activation

### 15. Security Testing
**Goal**: Verify security measures

- [ ] SQL injection prevention
- [ ] XSS prevention (content filtering)
- [ ] CSRF protection
- [ ] Rate limiting
- [ ] Password complexity requirements
- [ ] Token expiration enforcement
- [ ] Permission escalation attempts

## Test Execution Strategy

### Phase 1: Unit Tests (No External Dependencies)
- JWT service tests
- Permission bitmask tests
- Model tests
- Validation tests
- Error handling tests

### Phase 2: Integration Tests (Database Required)
- Repository layer tests
- Service layer tests
- Transaction tests

### Phase 3: API Tests (Full Stack)
- gRPC endpoint tests
- HTTP endpoint tests
- WebSocket tests
- End-to-end flows

### Phase 4: System Tests (Full Deployment)
- Docker compose deployment
- Multi-node cluster
- Load testing
- Streaming tests

## Test Environment Setup

### Minimum Requirements
```bash
# Install protoc
sudo apt-get install -y protobuf-compiler

# Setup test database
docker-compose up -d postgres redis

# Generate JWT keys
./scripts/generate-jwt-keys.sh

# Run migrations
cargo sqlx migrate run
```

### Configuration
```bash
export SYNCTV__DATABASE__URL="postgresql://synctv:synctv@localhost:5432/synctv"
export SYNCTV__REDIS__URL="redis://localhost:6379"
export SYNCTV__JWT__PRIVATE_KEY_PATH="./keys/jwt_private.pem"
export SYNCTV__JWT__PUBLIC_KEY_PATH="./keys/jwt_public.pem"
```

## Test Execution Commands

```bash
# Run all tests (excluding ignored)
cargo test --workspace

# Run all tests including database tests
cargo test --workspace -- --include-ignored

# Run specific test module
cargo test --package synctv-core --test integration_tests

# Run with logging
RUST_LOG=debug cargo test --workspace

# Run benchmarks
cargo bench --workspace
```

## Success Criteria

- All unit tests pass (100%)
- All integration tests pass (with database)
- All API endpoints respond correctly
- Server starts with all configuration variants
- No memory leaks in long-running tests
- Performance benchmarks meet targets
- Security vulnerabilities addressed
- Documentation matches implementation
