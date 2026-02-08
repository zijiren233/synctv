# Cluster Architecture and Deployment Patterns

## Overview

SyncTV supports both single-node and multi-node cluster deployments. This document explains the architecture, design decisions, and deployment patterns for running SyncTV in production.

## Deployment Modes

### Single-Node Mode

**Use Case**: Small deployments, development, testing

**Configuration**:
```yaml
# config.yaml
redis:
  url: ""  # Empty = single-node mode

# OR set environment variable:
SYNCTV__REDIS__URL=""
```

**Behavior**:
- All synchronization happens in-memory via `RoomMessageHub`
- No cross-node communication
- WebSocket connections handled by single server
- Simpler deployment and debugging

**Limitations**:
- No horizontal scalability
- Single point of failure
- Limited by single machine's resources

### Cluster Mode

**Use Case**: Production deployments, high availability, horizontal scaling

**Configuration**:
```yaml
# config.yaml
redis:
  url: "redis://redis.example.com:6379"
  key_prefix: "synctv:"
  cluster_mode: false  # Set true for Redis Cluster
```

**Behavior**:
- Events synchronized across nodes via Redis Pub/Sub
- WebSocket connections can connect to any node
- Automatic message deduplication
- Horizontal scalability

**Benefits**:
- High availability (node failures don't affect other nodes)
- Load balancing across multiple servers
- Geographic distribution possible

## Core Components

### 1. ClusterManager

**Purpose**: Unified interface for both local and distributed messaging

**Location**: `synctv-cluster/src/sync/cluster_manager.rs`

**Architecture**:
```rust
pub struct ClusterManager {
    // Local in-memory message hub
    local_hub: Arc<RoomMessageHub>,

    // Optional Redis pub/sub for cross-node sync
    redis_pubsub: Option<Arc<RedisPubSub>>,

    // Message deduplication to prevent echo
    deduplicator: MessageDeduplicator,

    // Node identifier for this instance
    node_id: String,
}
```

**Design Decision**: Same interface for both modes

The `ClusterManager` provides the same API regardless of deployment mode:

```rust
// Works in both single-node and cluster modes
cluster_manager.publish_room_event(room_id, event).await?;

// In single-node mode: Only local subscribers receive it
// In cluster mode: All nodes' subscribers receive it
```

**Rationale**:
- Application code doesn't need to know deployment mode
- Easy migration from single-node to cluster
- Simpler testing (can test without Redis)

### 2. RoomMessageHub

**Purpose**: Local in-memory message broadcasting

**Location**: `synctv-cluster/src/sync/room_hub.rs`

**Architecture**:
```rust
pub struct RoomMessageHub {
    // Map of room_id → list of subscribers
    subscribers: Arc<DashMap<String, Vec<Sender<RoomEvent>>>>,
}
```

**Responsibilities**:
- Subscribe to room events (returns a Stream)
- Publish events to local subscribers only
- Automatic cleanup of disconnected subscribers

**Usage**:
```rust
// Subscribe to room events
let mut stream = room_hub.subscribe(room_id).await;

while let Some(event) = stream.next().await {
    // Process event
    handle_room_event(event).await;
}
```

### 3. RedisPubSub

**Purpose**: Cross-node message synchronization via Redis

**Location**: `synctv-cluster/src/sync/redis_pubsub.rs`

**Architecture**:
```rust
pub struct RedisPubSub {
    redis_client: Arc<redis::Client>,
    key_prefix: String,
    subscriptions: Arc<DashMap<String, Vec<Sender<RoomEvent>>>>,
}
```

**Redis Channels**:
```
synctv:room:{room_id}:events  → Room-specific events
synctv:broadcast              → Global events
```

**Message Format**:
```json
{
  "node_id": "node-abc123",
  "message_id": "msg-xyz789",
  "timestamp": 1234567890,
  "event": { /* RoomEvent payload */ }
}
```

### 4. MessageDeduplicator

**Purpose**: Prevent receiving your own published messages

**Location**: `synctv-cluster/src/sync/cluster_manager.rs`

**Problem**: Redis Pub/Sub delivers messages to ALL subscribers, including the publisher

**Solution**: Track recently published message IDs and skip processing them

**Implementation**:
```rust
pub struct MessageDeduplicator {
    // LRU cache of recent message IDs
    seen_messages: Arc<Mutex<LruCache<String, ()>>>,

    // Time window for deduplication (default: 60s)
    dedup_window: Duration,
}
```

**Usage**:
```rust
// Before publishing
let message_id = MessageId::generate();
deduplicator.mark_sent(message_id);

// When receiving
if deduplicator.has_seen(message_id) {
    return; // Skip duplicate
}
```

### 5. ConnectionManager

**Purpose**: Track and limit concurrent connections

**Location**: `synctv-cluster/src/sync/connection_manager.rs`

**Architecture**:
```rust
pub struct ConnectionManager {
    // Active connections per user
    user_connections: Arc<DashMap<UserId, HashSet<ConnectionId>>>,

    // Active connections per room
    room_connections: Arc<DashMap<RoomId, HashSet<ConnectionId>>>,

    // Limits
    config: ConnectionLimits,
}
```

**Limits Enforced**:
- **Per-user limit**: Max connections per user (default: 10)
- **Per-room limit**: Max connections per room (default: 1000)
- **Global limit**: Total server connections (default: 100,000)
- **Idle timeout**: Auto-disconnect inactive connections (default: 30 min)
- **Max duration**: Force disconnect after duration (default: 24 hours)

**Cluster Consideration**:
- Each node tracks its own connections independently
- No cross-node connection tracking (intentional design)
- Limits are per-node, not global

**Rationale**: Simpler implementation, natural load distribution

## Event Flow

### Single-Node Event Flow

```
Client A → WebSocket → Server Node 1 → RoomMessageHub
                                           ↓
                                       All local subscribers
                                           ↓
                                   WebSocket → Clients A, B, C
```

### Cluster Event Flow

```
Client A → WebSocket → Server Node 1 → ClusterManager
                                           ├→ RoomMessageHub (local)
                                           │      ↓
                                           │  Local subscribers (Client A)
                                           │
                                           └→ RedisPubSub.publish()
                                                  ↓
                                              Redis Pub/Sub
                                                  ↓
                                         ┌────────┴────────┐
                                         ↓                 ↓
                                    Node 1 (skip)     Node 2 (receive)
                                                          ↓
                                                   RoomMessageHub
                                                          ↓
                                                  Remote subscribers
                                                          ↓
                                                   WebSocket → Clients B, C
```

## Scalability Characteristics

### Horizontal Scaling

**Add nodes by simply starting more instances:**

```bash
# Node 1
SYNCTV__REDIS__URL=redis://redis:6379 ./synctv-api

# Node 2
SYNCTV__REDIS__URL=redis://redis:6379 ./synctv-api

# Node 3
SYNCTV__REDIS__URL=redis://redis:6379 ./synctv-api
```

**Load balancing** via:
- Kubernetes Service
- NGINX/HAProxy
- Cloud load balancer (ALB, GCP LB)

### Performance Characteristics

**Single-Node**:
- **WebSocket capacity**: ~10,000-50,000 connections (depends on CPU/RAM)
- **Latency**: <5ms (local memory)
- **Throughput**: ~100,000 events/second

**Cluster (3 nodes)**:
- **WebSocket capacity**: ~30,000-150,000 connections (scales linearly)
- **Latency**: ~10-30ms (includes Redis round-trip)
- **Throughput**: ~50,000 events/second (bottleneck: Redis Pub/Sub)

### Redis as Bottleneck

**Redis Pub/Sub limitations**:
- Single-threaded message delivery
- No persistence (fire-and-forget)
- No backpressure

**Mitigation strategies**:
1. **Use Redis Cluster** for higher throughput
2. **Batch events** when possible to reduce messages
3. **Partition rooms** across Redis instances
4. **Use Redis Streams** for critical events (future work)

## Failure Scenarios

### Scenario 1: Node Failure

**Problem**: Node crashes or becomes unreachable

**Behavior**:
1. Clients connected to failed node are disconnected
2. Other nodes continue operating normally
3. Load balancer redirects new connections to healthy nodes

**Recovery**:
- Clients auto-reconnect via WebSocket reconnect logic
- Reconnections distributed to healthy nodes
- No data loss (room state in PostgreSQL)

**Recommendation**: Use at least 3 nodes for production

### Scenario 2: Redis Failure

**Problem**: Redis becomes unavailable

**Behavior**:
1. Nodes detect Redis connection failure
2. **Automatic fallback to single-node mode per instance**
3. Each node continues serving its connected clients
4. Cross-node events STOP (rooms effectively isolated per node)

**Impact**:
- Clients on same node can still sync with each other
- Clients on different nodes see different room states
- New connections work but are isolated per node

**Recovery**:
1. Redis comes back online
2. Nodes automatically reconnect
3. Cross-node sync resumes
4. Room states eventually converge

**Recommendation**: Use Redis Sentinel or Redis Cluster for HA

### Scenario 3: Network Partition (Split Brain)

**Problem**: Network partition separates nodes

**Behavior**:
- Nodes on different sides of partition operate independently
- Room states diverge
- Clients see different views based on connected node

**Resolution**:
- **Last-write-wins** for playback state (using version numbers)
- **Merge on partition heal** (future: CRDTs)

**Recommendation**: Use network topology awareness, avoid cross-datacenter clusters without proper setup

## Monitoring and Observability

### Metrics

**Exposed via Prometheus** (`/metrics` endpoint):

```prometheus
# Connection metrics
synctv_connections_total{node="node1"}
synctv_connections_per_room{room="room123",node="node1"}
synctv_connections_per_user{user="user456",node="node1"}

# Event metrics
synctv_events_published_total{room="room123",type="playback_state"}
synctv_events_received_total{node="node1"}
synctv_events_deduplicated_total{node="node1"}

# Redis metrics
synctv_redis_pubsub_messages_total{channel="room:*"}
synctv_redis_connection_errors_total{node="node1"}

# Performance metrics
synctv_event_publish_duration_seconds{quantile="0.99"}
synctv_event_delivery_duration_seconds{quantile="0.99"}
```

### Health Checks

**Liveness probe**: `GET /health`
```json
{
  "status": "ok",
  "node_id": "node-abc123",
  "uptime_seconds": 3600
}
```

**Readiness probe**: `GET /ready`
```json
{
  "status": "ready",
  "checks": {
    "database": "ok",
    "redis": "ok",
    "cluster_mode": true
  }
}
```

### Logging

**Structured logging with correlation IDs**:

```rust
tracing::info!(
    node_id = %self.node_id,
    room_id = %room_id,
    event_type = ?event.event_type(),
    "Publishing room event"
);
```

## Best Practices

### 1. Use Connection Limits

```yaml
cluster:
  connection_limits:
    per_user: 10
    per_room: 1000
    global: 100000
    idle_timeout_secs: 1800
    max_duration_secs: 86400
```

### 2. Enable Redis Persistence

```redis.conf
# Use AOF for durability
appendonly yes
appendfsync everysec
```

### 3. Monitor Redis Memory

```bash
# Set max memory limit
maxmemory 2gb
maxmemory-policy allkeys-lru
```

### 4. Use Health Checks

```yaml
# Kubernetes example
livenessProbe:
  httpGet:
    path: /health
    port: 8080
  periodSeconds: 10

readinessProbe:
  httpGet:
    path: /ready
    port: 8080
  periodSeconds: 5
```

### 5. Implement Graceful Shutdown

```rust
// On SIGTERM
async fn shutdown(cluster_manager: Arc<ClusterManager>) {
    // Stop accepting new connections
    server.graceful_shutdown();

    // Notify clients to reconnect
    cluster_manager.broadcast_shutdown_notice().await;

    // Wait for clients to disconnect
    tokio::time::sleep(Duration::from_secs(30)).await;

    // Close connections
    cluster_manager.close_all_connections().await;
}
```

## Future Improvements

### Planned Enhancements

1. **Service Discovery**
   - Automatic node registration/deregistration
   - Health-based load balancing
   - Dynamic node addition without config changes

2. **Distributed State Synchronization**
   - CRDT-based room state for convergence
   - Conflict resolution for split-brain scenarios
   - Optimistic concurrency control

3. **Cache Coherency Protocol**
   - Distributed cache invalidation
   - Cache warming on node startup
   - Consistent hashing for cache distribution

4. **Connection Migration**
   - Transfer connections to other nodes on failure
   - Session persistence across reconnects
   - Zero-downtime rolling updates

5. **Advanced Redis Integration**
   - Use Redis Streams for event persistence
   - Implement consumer groups for fan-out
   - Add replay capability for missed events

### Current Limitations

1. **No Service Discovery**: Nodes don't know about each other (only via Redis)
2. **No Automatic Failover**: Requires external orchestration (K8s, Nomad)
3. **Limited Split-Brain Handling**: Manual intervention may be needed
4. **Per-Node Connection Tracking**: Global limits not enforced
5. **Fire-and-Forget Events**: No delivery guarantees

## Deployment Examples

### Docker Compose (Single-Node)

```yaml
version: '3.8'
services:
  synctv:
    image: synctv:latest
    ports:
      - "8080:8080"
      - "50051:50051"
    environment:
      - SYNCTV__DATABASE__URL=postgresql://postgres:postgres@db:5432/synctv
      - SYNCTV__REDIS__URL=  # Empty for single-node
```

### Kubernetes (Cluster Mode)

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: synctv-api
spec:
  replicas: 3  # Multi-node cluster
  selector:
    matchLabels:
      app: synctv-api
  template:
    metadata:
      labels:
        app: synctv-api
    spec:
      containers:
      - name: synctv-api
        image: synctv:latest
        env:
        - name: SYNCTV__DATABASE__URL
          value: "postgresql://postgres:postgres@postgres:5432/synctv"
        - name: SYNCTV__REDIS__URL
          value: "redis://redis:6379"
        - name: NODE_NAME
          valueFrom:
            fieldRef:
              fieldPath: spec.nodeName
        ports:
        - containerPort: 8080
          name: http
        - containerPort: 50051
          name: grpc
        livenessProbe:
          httpGet:
            path: /health
            port: 8080
          periodSeconds: 10
        readinessProbe:
          httpGet:
            path: /ready
            port: 8080
          periodSeconds: 5
        resources:
          requests:
            memory: "512Mi"
            cpu: "250m"
          limits:
            memory: "2Gi"
            cpu: "1000m"

---
apiVersion: v1
kind: Service
metadata:
  name: synctv-api
spec:
  type: LoadBalancer
  selector:
    app: synctv-api
  ports:
  - port: 80
    targetPort: 8080
    name: http
  - port: 50051
    targetPort: 50051
    name: grpc
```

## Related Files

- **ClusterManager**: `synctv-cluster/src/sync/cluster_manager.rs`
- **RoomMessageHub**: `synctv-cluster/src/sync/room_hub.rs`
- **RedisPubSub**: `synctv-cluster/src/sync/redis_pubsub.rs`
- **ConnectionManager**: `synctv-cluster/src/sync/connection_manager.rs`
- **Server Initialization**: `synctv/src/main.rs:106-140`
- **Configuration**: `synctv-core/src/config.rs`

## Summary

**Key Takeaways:**

1. SyncTV supports both single-node and cluster modes transparently
2. ClusterManager provides unified API for both modes
3. Redis Pub/Sub enables cross-node synchronization
4. MessageDeduplicator prevents event echo
5. ConnectionManager enforces per-node limits
6. Horizontal scaling achieved by adding more nodes
7. Redis failure causes automatic fallback to single-node behavior per instance
8. Future enhancements planned for true service discovery and HA
