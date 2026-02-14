# SyncTV - Rust Implementation

A production-grade real-time synchronized video watching platform built in Rust.

## Features

- **Real-time Synchronization**: Watch videos together with friends in perfect sync
- **Multi-Provider Support**: Bilibili, Alist, Emby, and direct URLs
- **Live Streaming**: RTMP push/pull with HLS and FLV support
- **Horizontal Scalability**: Kubernetes-ready multi-replica deployment
- **High Performance**: Built with Rust for maximum efficiency
- **Type Safety**: Compile-time guarantees and zero-cost abstractions

## Architecture

- **synctv-core**: Core business logic library
- **synctv-api**: gRPC + HTTP API service
- **synctv-livestream**: Live streaming service (RTMP/HLS/FLV)
- **synctv-cluster**: Cluster coordination library
- **synctv-xiu**: Consolidated streaming library (RTMP/HLS/HTTP-FLV protocols)

## Quick Start

### Prerequisites

- Rust 1.75+ (2021 edition)
- PostgreSQL 14+
- Redis 7+
- OpenSSL

### 1. Generate JWT Keys

```bash
./scripts/generate-jwt-keys.sh
```

This creates `keys/jwt_private.pem` and `keys/jwt_public.pem`.

### 2. Set Environment Variables

```bash
# Database
export SYNCTV__DATABASE__URL="postgresql://synctv:synctv@localhost:5432/synctv"

# Redis
export SYNCTV__REDIS__URL="redis://localhost:6379"

# JWT Keys
export SYNCTV__JWT__PRIVATE_KEY_PATH="./keys/jwt_private.pem"
export SYNCTV__JWT__PUBLIC_KEY_PATH="./keys/jwt_public.pem"

# Server
export SYNCTV__SERVER__GRPC_PORT=50051
export SYNCTV__SERVER__HTTP_PORT=8080
```

### 3. Run Database Migrations

```bash
cargo install sqlx-cli --no-default-features --features postgres
sqlx migrate run --database-url $SYNCTV__DATABASE__URL
```

### 4. Start the Server

```bash
# Set JWT secret (required for production)
export SYNCTV__JWT__SECRET="your-secure-random-string-at-least-32-chars"

cargo run --bin synctv
```

The gRPC server will start on `0.0.0.0:50051` and HTTP on `0.0.0.0:8080`.

## Development

### Run Tests

```bash
cargo test --workspace
```

### Run with Logging

```bash
RUST_LOG=debug cargo run --bin synctv
```

### Build Release

```bash
cargo build --release --workspace
```

## API

### gRPC API

Use gRPC reflection to explore the API:

```bash
grpcurl -plaintext localhost:50051 list
grpcurl -plaintext localhost:50051 list synctv.client.ClientService
```

### Example: Register User

```bash
grpcurl -plaintext -d '{
  "username": "alice",
  "email": "alice@example.com",
  "password": "securepassword123"
}' localhost:50051 synctv.client.ClientService/Register
```

### Example: Login

```bash
grpcurl -plaintext -d '{
  "username": "alice",
  "password": "securepassword123"
}' localhost:50051 synctv.client.ClientService/Login
```

## Configuration

Configuration can be provided via:
1. Environment variables (highest priority): `SYNCTV__SECTION__KEY`
2. Config file: `config.toml` or `config.yaml`
3. Defaults (lowest priority)

Example `config.toml`:

```toml
[server]
host = "0.0.0.0"
grpc_port = 50051
http_port = 8080
enable_reflection = true

[database]
url = "postgresql://synctv:synctv@localhost:5432/synctv"
max_connections = 20
min_connections = 5

[redis]
url = "redis://localhost:6379"
key_prefix = "synctv:"

[jwt]
private_key_path = "./keys/jwt_private.pem"
public_key_path = "./keys/jwt_public.pem"

[logging]
level = "info"
format = "pretty"  # or "json"
```

## Security

- **Password Hashing**: Argon2id (PHC 2023 winner)
- **JWT**: RS256 asymmetric encryption
- **Permissions**: 64-bit bitmask system
- **TLS**: Recommended for production

## License

MIT OR Apache-2.0

## Contributing

Contributions are welcome! Please read CONTRIBUTING.md for guidelines.

## Status

**Current Status**: Production-ready core features

### Completed Features
- [x] User authentication (registration, login, JWT tokens)
- [x] Room management and real-time synchronization
- [x] Multi-provider media support (Bilibili, Alist, Emby)
- [x] Live streaming (RTMP push, HLS/FLV playback)
- [x] Multi-replica cluster support
- [x] OAuth2 integration (GitHub, Google, OIDC)
- [x] Permission system with 64-bit bitmask
- [x] WebSocket real-time communication

### In Progress
- [ ] WebRTC SFU for large rooms
- [ ] Cross-replica cache invalidation via Redis Pub/Sub

**Next Milestone**: Production hardening and performance optimization
