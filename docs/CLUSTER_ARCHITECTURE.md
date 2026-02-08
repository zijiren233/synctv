# SyncTV Cluster Architecture Analysis

## Executive Summary

This document analyzes the current SyncTV architecture and provides a comprehensive plan for evolving it into a production-ready distributed cluster system. Currently, SyncTV is designed for single-node deployment with vendor backend clustering support, but lacks cross-instance synchronization for room state.

## Current Architecture

### System Components

```
┌─────────────────────────────────────────────────────────┐
│                    SyncTV Instance                       │
│  ┌──────────────────────────────────────────────────┐  │
│  │              HTTP/WebSocket Server                │  │
│  │           (Gin + Gorilla WebSocket)              │  │
│  └────────────────────┬─────────────────────────────┘  │
│                       │                                  │
│  ┌────────────────────┴─────────────────────────────┐  │
│  │              In-Memory Room Hub                   │  │
│  │  • Per-room WebSocket client tracking            │  │
│  │  • Room state (current movie, playlist)          │  │
│  │  • Broadcast within single instance only         │  │
│  └────────────────────┬─────────────────────────────┘  │
│                       │                                  │
│  ┌────────────────────┴─────────────────────────────┐  │
│  │         Database Layer (GORM)                     │  │
│  │  • User/Room persistence                         │  │
│  │  • Settings & configuration                       │  │
│  │  • Vendor credentials                            │  │
│  └──────────────────────────────────────────────────┘  │
└───────────────────────┬─────────────────────────────────┘
                        │
        ┌───────────────┼───────────────┐
        │               │               │
    ┌───────┐      ┌────────┐    ┌──────────┐
    │SQLite │      │Vendor  │    │Service   │
    │MySQL  │      │Backends│    │Discovery │
    │Postgres│     │(gRPC)  │    │(Consul/  │
    └───────┘      └────────┘    │etcd)     │
                                 └──────────┘
```

### Key Features Implemented
- ✅ HTTP REST API with JWT authentication
- ✅ WebSocket real-time communication (per-instance)
- ✅ Multiple database support (SQLite, MySQL, PostgreSQL)
- ✅ OAuth2 plugin system with extensibility
- ✅ Vendor backends (Bilibili, Alist, Emby) with gRPC
- ✅ Service discovery for vendor backends (Consul/etcd)
- ✅ RTMP streaming support (muxed with HTTP)
- ✅ Rate limiting and CORS middleware
- ✅ Graceful shutdown via sysnotify
- ✅ Health check endpoints

### Critical Limitations for Cluster Deployment

#### 1. In-Memory Room State
**Location**: `internal/op/hub.go`, `internal/op/room.go`

Each SyncTV instance maintains its own Hub per room:
```go
type Hub struct {
    broadcast chan *broadcastMessage
    exit      chan struct{}
    id        string
    clients   rwmap.RWMap[string, *clients]  // ⚠️ Local only
    wg        sync.WaitGroup
    once      utils.Once
    closed    uint32
}
```

**Problem**: When User A on Instance 1 sends a message, User B on Instance 2 won't receive it.

#### 2. No Cross-Instance Messaging
**Current**: `Hub.Broadcast()` only sends to clients connected to the same instance.

**Missing**: Message bus (Redis Pub/Sub, NATS, etc.) for cross-instance communication.

#### 3. No Distributed Session Management
**Current**: WebSocket connections are tied to the instance they connect to.

**Problem**: Load balancer must use sticky sessions, preventing efficient load distribution.

#### 4. Race Conditions in Multi-Instance Writes
**Scenario**: Two instances simultaneously updating the same room's current movie.

**Current**: No distributed locking mechanism.

#### 5. Service Discovery Only for Vendor Backends
**Location**: `internal/vendor/vendor.go` (uses Kratos + Consul/etcd)

**Problem**: This infrastructure exists but is only used for vendor backends, not for SyncTV-to-SyncTV communication.

## Production Cluster Architecture (Proposed)

### Target Architecture

```
┌─────────────────────────────────────────────────────────┐
│                  Load Balancer (nginx/HAProxy)          │
│              (WebSocket sticky sessions optional)       │
└──────────────┬──────────────┬──────────────┬────────────┘
               │              │              │
     ┌─────────┴────┐  ┌─────┴────┐  ┌─────┴────┐
     │ Instance 1   │  │Instance 2│  │Instance 3│
     │ (Stateless)  │  │(Stateless)│  │(Stateless)│
     └──────┬───────┘  └─────┬────┘  └─────┬────┘
            │                │              │
            └────────────────┼──────────────┘
                             │
          ┌──────────────────┴──────────────────┐
          │         Message Bus (Redis)         │
          │   • Pub/Sub for room broadcasts     │
          │   • Presence tracking (online users)│
          │   • Distributed locks               │
          └──────────────────┬──────────────────┘
                             │
          ┌──────────────────┴──────────────────┐
          │      Session Store (Redis)          │
          │   • WebSocket connection metadata   │
          │   • Room membership cache           │
          └──────────────────┬──────────────────┘
                             │
          ┌──────────────────┴──────────────────┐
          │   Database (Primary + Replicas)     │
          │   • User/Room persistence           │
          │   • Settings                        │
          └─────────────────────────────────────┘
```

### Design Principles

1. **Stateless Instances**: All instances are identical and interchangeable
2. **Shared Nothing**: No shared memory between instances
3. **Event-Driven**: Use pub/sub for cross-instance communication
4. **Eventually Consistent**: Accept slight delays in propagation
5. **Fault Tolerant**: Instance failure doesn't affect other instances

## Implementation Roadmap

### Phase 1: Observability Foundation (COMPLETED ✅)
- [x] Add health check endpoints (`/health`, `/ready`, `/live`)
- [ ] Add Prometheus metrics endpoint
- [ ] Add structured logging with trace IDs
- [ ] Add performance profiling endpoints (pprof)

### Phase 2: Distributed State Management (HIGH PRIORITY)

#### 2.1 Redis Integration
**Goal**: Add Redis as a shared state store and message bus.

**Implementation**:
1. Add Redis client dependency (go-redis/redis)
2. Create `internal/pubsub/` package:
   ```go
   type PubSub interface {
       Publish(ctx context.Context, channel string, message []byte) error
       Subscribe(ctx context.Context, channels ...string) (<-chan Message, error)
       Unsubscribe(ctx context.Context, channels ...string) error
   }
   ```

3. Implement Redis adapter:
   ```go
   type RedisPubSub struct {
       client *redis.Client
   }
   ```

4. Add configuration:
   ```yaml
   redis:
     enabled: true
     host: localhost
     port: 6379
     password: ""
     db: 0
     pool_size: 10
   ```

#### 2.2 Distributed Hub
**Goal**: Make Hub broadcast across all instances.

**Changes to `internal/op/hub.go`**:
```go
type Hub struct {
    broadcast    chan *broadcastMessage
    exit         chan struct{}
    id           string
    clients      rwmap.RWMap[string, *clients]
    wg           sync.WaitGroup
    once         utils.Once
    closed       uint32
    pubsub       pubsub.PubSub  // NEW: Redis pub/sub
    instanceID   string          // NEW: Unique instance ID
}

func (h *Hub) Broadcast(data Message, conf ...BroadcastConf) error {
    // 1. Broadcast to local clients
    h.broadcastLocal(data, conf...)

    // 2. Publish to Redis for other instances
    if h.pubsub != nil {
        msg := &ClusterMessage{
            InstanceID: h.instanceID,
            RoomID:     h.id,
            Data:       data,
            Config:     conf,
        }
        return h.pubsub.Publish(ctx, "room:"+h.id, msg)
    }

    return nil
}

func (h *Hub) subscribeToCluster() {
    messages, err := h.pubsub.Subscribe(ctx, "room:"+h.id)
    if err != nil {
        log.Error("Failed to subscribe to cluster channel")
        return
    }

    for msg := range messages {
        // Ignore messages from this instance
        if msg.InstanceID == h.instanceID {
            continue
        }

        // Broadcast to local clients
        h.broadcastLocal(msg.Data, msg.Config...)
    }
}
```

#### 2.3 Distributed Presence Tracking
**Goal**: Track which users are in which rooms across all instances.

**Implementation** (`internal/presence/`):
```go
type Presence interface {
    // Join marks a user as present in a room
    Join(ctx context.Context, roomID, userID, instanceID string) error

    // Leave removes user from room presence
    Leave(ctx context.Context, roomID, userID string) error

    // GetRoomMembers returns all users currently in the room
    GetRoomMembers(ctx context.Context, roomID string) ([]UserPresence, error)

    // GetUserLocation returns which instance the user is connected to
    GetUserLocation(ctx context.Context, roomID, userID string) (string, error)
}

type UserPresence struct {
    UserID      string
    InstanceID  string
    ConnectedAt time.Time
}
```

**Redis Implementation**:
- Use Redis ZADD with timestamp for automatic expiry
- Key pattern: `presence:room:{roomID}` -> ZSET of userID:instanceID
- TTL heartbeat every 30 seconds

### Phase 3: Distributed Locking (MEDIUM PRIORITY)

#### 3.1 Redlock Implementation
**Goal**: Prevent race conditions for room state updates.

**Use Cases**:
- Updating current movie
- Adding/removing movies from playlist
- Modifying room settings

**Implementation** (`internal/lock/`):
```go
type DistributedLock interface {
    Acquire(ctx context.Context, key string, ttl time.Duration) (Lock, error)
    Release(ctx context.Context, lock Lock) error
}

type Lock struct {
    Key      string
    Value    string  // Unique token
    Instance string
    ExpireAt time.Time
}
```

**Integration Example**:
```go
func (r *Room) UpdateMovie(movieID string, movie *model.MovieBase) error {
    lock, err := r.lock.Acquire(ctx, "room:"+r.ID+":movie", 5*time.Second)
    if err != nil {
        return errors.Wrap(err, "failed to acquire lock")
    }
    defer r.lock.Release(ctx, lock)

    // Perform update
    return r.updateMovieInternal(movieID, movie)
}
```

### Phase 4: Session Affinity (OPTIONAL)

#### 4.1 Sticky Sessions
**Goal**: Route users to the same instance when possible.

**Options**:
1. **Cookie-based**: Load balancer sets cookie on first connection
2. **IP-based**: Hash client IP to instance (can cause imbalance)
3. **User-ID-based**: Hash user ID for logged-in users

**nginx Configuration Example**:
```nginx
upstream synctv {
    ip_hash;  # or hash $cookie_synctv_instance consistent;
    server 10.0.0.1:8080;
    server 10.0.0.2:8080;
    server 10.0.0.3:8080;
}
```

**Pros**: Reduces Redis traffic, simpler failover
**Cons**: Uneven load distribution, complicated failover

**Recommendation**: Implement distributed state first, add sticky sessions only if Redis becomes a bottleneck.

### Phase 5: Metrics and Monitoring (HIGH PRIORITY)

#### 5.1 Prometheus Metrics
**Endpoint**: `/metrics`

**Key Metrics**:
```go
// Business metrics
room_active_total          // Number of rooms with active connections
room_viewers_total         // Total viewers across all rooms
room_messages_sent_total   // Messages broadcast (counter)
room_messages_received_total

// Technical metrics
http_requests_total{method,path,status}
http_request_duration_seconds{method,path}
websocket_connections_active
websocket_messages_sent_total
websocket_messages_received_total
db_connections_active
db_connections_idle
db_query_duration_seconds{operation}
redis_commands_total{command}
redis_command_duration_seconds{command}

// Instance metrics
instance_uptime_seconds
instance_memory_bytes
instance_goroutines_total
```

**Implementation** (`internal/metrics/`):
```go
package metrics

import "github.com/prometheus/client_golang/prometheus"

var (
    RoomViewers = prometheus.NewGaugeVec(
        prometheus.GaugeOpts{
            Name: "synctv_room_viewers_total",
            Help: "Number of viewers in each room",
        },
        []string{"room_id"},
    )

    MessagesSent = prometheus.NewCounterVec(
        prometheus.CounterOpts{
            Name: "synctv_messages_sent_total",
            Help: "Total messages sent",
        },
        []string{"room_id", "type"},
    )
)

func init() {
    prometheus.MustRegister(RoomViewers, MessagesSent)
}
```

#### 5.2 Distributed Tracing
**Library**: OpenTelemetry

**Trace Spans**:
- HTTP request → Handler → Database query
- WebSocket message → Hub broadcast → Redis publish
- User action → All affected operations

**Implementation**:
```go
// Add trace context to bootstrap
func InitTracing(ctx context.Context) error {
    tp := sdktrace.NewTracerProvider(
        sdktrace.WithBatcher(exporter),
        sdktrace.WithResource(resource.NewWithAttributes(
            semconv.ServiceNameKey.String("synctv"),
            semconv.ServiceInstanceIDKey.String(instanceID),
        )),
    )
    otel.SetTracerProvider(tp)
    return nil
}
```

### Phase 6: Performance Optimizations

#### 6.1 Connection Pooling
- Database: Already implemented (GORM)
- Redis: Add connection pool (go-redis default is 10)

#### 6.2 Message Batching
**Goal**: Reduce Redis pub/sub overhead.

**Implementation**:
- Buffer messages for 10ms or 100 messages (whichever comes first)
- Batch publish to Redis
- Unbatch on receiving instance

#### 6.3 Caching Strategy
**What to Cache**:
- User permissions (5-minute TTL)
- Room settings (1-minute TTL)
- Active room list (30-second TTL)

**What NOT to Cache**:
- Current movie state (must be real-time)
- WebSocket connections (too volatile)

## Migration Strategy

### Zero-Downtime Migration Plan

#### Step 1: Feature Flags
Add configuration to toggle cluster features:
```yaml
cluster:
  enabled: false          # Feature flag for entire cluster mode
  redis:
    enabled: false
    host: localhost
    port: 6379
  pubsub:
    enabled: false
  presence:
    enabled: false
```

#### Step 2: Shadow Mode
Deploy Redis infrastructure but don't rely on it:
- Publish to Redis but also keep local broadcast
- Read from Redis but don't fail if unavailable
- Log discrepancies for monitoring

#### Step 3: Gradual Rollout
1. Enable on one instance (canary)
2. Monitor metrics for 24 hours
3. Enable on 50% of instances
4. Monitor for another 24 hours
5. Full rollout

#### Step 4: Fallback Plan
If issues occur:
1. Disable cluster mode via config
2. Restart instances
3. All instances revert to local-only mode

## Configuration Changes

### New Configuration Structure

```yaml
# Current configuration (keep backward compatible)
server:
  http:
    listen: 0.0.0.0
    port: 8080
  rtmp:
    enable: true

database:
  type: sqlite3
  name: synctv.db

# NEW: Cluster configuration
cluster:
  enabled: false
  instance_id: ""  # Auto-generate if empty

  redis:
    enabled: false
    addresses:
      - localhost:6379
    password: ""
    db: 0
    pool_size: 10
    max_retries: 3
    dial_timeout: 5s
    read_timeout: 3s
    write_timeout: 3s

  pubsub:
    enabled: false
    channels_buffer_size: 100
    message_batch_size: 10
    message_batch_timeout: 10ms

  presence:
    enabled: false
    heartbeat_interval: 30s
    ttl: 60s

  lock:
    enabled: false
    default_ttl: 5s
    retry_count: 3
    retry_delay: 100ms

# NEW: Observability configuration
observability:
  metrics:
    enabled: true
    path: /metrics
  tracing:
    enabled: false
    endpoint: ""
    sampler_ratio: 0.1
  profiling:
    enabled: false
    path: /debug/pprof
```

## Testing Strategy

### Unit Tests
- [ ] Redis pub/sub adapter
- [ ] Presence tracking logic
- [ ] Distributed lock implementation
- [ ] Metrics collection

### Integration Tests
- [ ] Multi-instance broadcast
- [ ] User presence across instances
- [ ] Lock contention scenarios
- [ ] Redis failover handling

### Load Tests
**Tool**: k6 or Gatling

**Scenarios**:
1. **Room Join Storm**: 1000 users join same room simultaneously
2. **Message Flood**: 100 messages/second in single room
3. **Room Creation**: 100 rooms created per second
4. **Multi-Room**: 10 rooms with 100 users each

**Success Criteria**:
- 95th percentile latency < 100ms
- 99th percentile latency < 500ms
- 0% message loss
- Graceful degradation under overload

### Chaos Engineering
**Tool**: Chaos Mesh (Kubernetes)

**Experiments**:
1. Kill random instance (verify failover)
2. Inject 500ms network latency to Redis
3. Fill disk on database node
4. Simulate Redis split-brain

## Security Considerations

### 1. Redis Security
- **Authentication**: Always set `requirepass`
- **TLS**: Enable TLS for Redis connections in production
- **Network**: Bind Redis to private network only
- **ACLs**: Use Redis 6+ ACLs to restrict commands

### 2. Instance-to-Instance Communication
Currently vendor backends use JWT middleware. Extend this to SyncTV-to-SyncTV:
```go
// Existing: internal/vendor/vendor.go uses JWT middleware
// Extend to: internal/cluster/auth.go
```

### 3. Rate Limiting
**Current**: Per-instance rate limiting (can be bypassed by hitting multiple instances)

**Solution**: Move rate limiting to Redis:
```go
// Use Redis INCR with EXPIRE
func (rl *RedisRateLimiter) Allow(ctx context.Context, key string) bool {
    count, err := rl.client.Incr(ctx, "ratelimit:"+key).Result()
    if err != nil {
        return true  // Fail open
    }

    if count == 1 {
        rl.client.Expire(ctx, "ratelimit:"+key, rl.window)
    }

    return count <= rl.limit
}
```

## Deployment Recommendations

### Kubernetes Deployment

#### 1. StatefulSet vs Deployment
**Recommendation**: Use **Deployment** (stateless instances)

Rationale:
- All state is in Redis/Database
- Instances are interchangeable
- Easy horizontal scaling

#### 2. Resource Requests/Limits
```yaml
resources:
  requests:
    memory: "512Mi"
    cpu: "250m"
  limits:
    memory: "1Gi"
    cpu: "1000m"
```

#### 3. Health Probes
```yaml
livenessProbe:
  httpGet:
    path: /live
    port: 8080
  initialDelaySeconds: 10
  periodSeconds: 10
  timeoutSeconds: 5
  failureThreshold: 3

readinessProbe:
  httpGet:
    path: /ready
    port: 8080
  initialDelaySeconds: 5
  periodSeconds: 5
  timeoutSeconds: 3
  failureThreshold: 2
```

#### 4. Horizontal Pod Autoscaler
```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: synctv-hpa
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: synctv
  minReplicas: 3
  maxReplicas: 10
  metrics:
  - type: Resource
    resource:
      name: cpu
      target:
        type: Utilization
        averageUtilization: 70
  - type: Pods
    pods:
      metric:
        name: synctv_websocket_connections_active
      target:
        type: AverageValue
        averageValue: "1000"
```

### Docker Compose (Development)

```yaml
version: '3.8'

services:
  synctv-1:
    build: .
    ports:
      - "8081:8080"
    environment:
      - SYNCTV_CLUSTER_ENABLED=true
      - SYNCTV_CLUSTER_REDIS_ENABLED=true
      - SYNCTV_CLUSTER_REDIS_ADDRESSES=redis:6379
    depends_on:
      - redis
      - postgres

  synctv-2:
    build: .
    ports:
      - "8082:8080"
    environment:
      - SYNCTV_CLUSTER_ENABLED=true
      - SYNCTV_CLUSTER_REDIS_ENABLED=true
      - SYNCTV_CLUSTER_REDIS_ADDRESSES=redis:6379
    depends_on:
      - redis
      - postgres

  redis:
    image: redis:7-alpine
    command: redis-server --requirepass yourpassword
    ports:
      - "6379:6379"
    volumes:
      - redis-data:/data

  postgres:
    image: postgres:15-alpine
    environment:
      POSTGRES_PASSWORD: yourpassword
      POSTGRES_DB: synctv
    ports:
      - "5432:5432"
    volumes:
      - postgres-data:/var/lib/postgresql/data

  nginx:
    image: nginx:alpine
    ports:
      - "8080:80"
    volumes:
      - ./nginx.conf:/etc/nginx/nginx.conf
    depends_on:
      - synctv-1
      - synctv-2

volumes:
  redis-data:
  postgres-data:
```

## Performance Expectations

### Single Instance (Current)
- **Concurrent Users**: 1,000-5,000 per instance
- **Rooms**: 100-500 active rooms
- **Messages/sec**: 1,000-5,000

### Cluster (3 instances)
- **Concurrent Users**: 10,000-15,000 total
- **Rooms**: 500-1,500 active rooms
- **Messages/sec**: 10,000-20,000
- **Latency Overhead**: +10-50ms (Redis network overhead)

### Bottlenecks
1. **Redis Pub/Sub**: 50,000 messages/sec per Redis instance
2. **Database**: Write-heavy workload, consider read replicas
3. **WebSocket Connections**: File descriptor limits (ulimit -n)

## Cost Analysis

### Infrastructure Costs (AWS, monthly estimates)

#### Current (Single Instance)
- EC2 t3.medium: $30
- RDS db.t3.small: $30
- Total: **$60/month**

#### Cluster (3 instances)
- 3x EC2 t3.medium: $90
- RDS db.t3.medium: $60
- ElastiCache Redis (cache.t3.small): $25
- Load Balancer (ALB): $20
- Total: **$195/month**

**ROI**: Support 3-5x more users for 3x cost = cost-effective

## Conclusion

The current SyncTV architecture is well-designed for single-instance deployment with excellent code quality and separation of concerns. However, it lacks the distributed state management required for true cluster deployment.

**Recommended Immediate Actions**:
1. ✅ Add health check endpoints (COMPLETED)
2. Add Prometheus metrics
3. Add Redis integration with feature flags
4. Implement distributed Hub broadcast
5. Add presence tracking
6. Comprehensive testing

**Long-term Vision**:
Build a production-ready, horizontally-scalable platform that can serve 10,000+ concurrent users across multiple geographic regions with <100ms latency.

**Effort Estimate**:
- Phase 1 (Observability): 1-2 weeks (partially complete)
- Phase 2 (Distributed State): 3-4 weeks
- Phase 3 (Distributed Locking): 1-2 weeks
- Phase 4 (Session Affinity): 1 week (optional)
- Phase 5 (Metrics): 1-2 weeks
- Testing & Documentation: 2 weeks
- **Total**: 9-13 weeks for full cluster support

---

*Document Version: 1.0*
*Last Updated: 2026-02-08*
*Author: Architecture Analysis Agent*
