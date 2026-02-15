# SyncTV Helm Chart

A production-ready Helm chart for deploying SyncTV - a distributed video synchronization platform with real-time streaming capabilities (RTMP/HLS/WebRTC).

## Features

- **API Deployment**: Stateless HTTP/gRPC API servers with horizontal auto-scaling
- **Streaming StatefulSet**: Stateful RTMP/HLS/FLV streaming servers with persistent storage
- **Multi-Replica**: Native cluster support with automatic node discovery and load balancing
- **High Availability**: Pod Disruption Budgets, anti-affinity rules, health checks
- **Security**: Network policies, pod security contexts, secrets management
- **Observability**: Prometheus metrics, structured logging
- **Production-Ready**: Resource limits, autoscaling, ingress with TLS

## Prerequisites

- Kubernetes 1.23+
- Helm 3.8+
- PV provisioner support in the underlying infrastructure (for streaming persistence)
- Ingress controller (nginx recommended)
- cert-manager (optional, for automatic TLS certificates)
- PostgreSQL 14+ (external or via Bitnami chart)
- Redis 7+ (external or via Bitnami chart)

## Installation

### 1. Add Helm Repository (if published)

```bash
helm repo add synctv https://charts.synctv.example.com
helm repo update
```

### 2. Install External Dependencies

#### PostgreSQL

```bash
helm repo add bitnami https://charts.bitnami.com/bitnami

helm install postgresql bitnami/postgresql \
  --namespace synctv \
  --create-namespace \
  --set auth.username=synctv \
  --set auth.password=your-secure-password \
  --set auth.database=synctv \
  --set primary.persistence.size=20Gi
```

#### Redis

```bash
helm install redis bitnami/redis \
  --namespace synctv \
  --set auth.password=your-redis-password \
  --set master.persistence.size=8Gi
```

### 3. Create Custom Values File

Create a `my-values.yaml` file:

```yaml
# Basic Configuration
api:
  ingress:
    enabled: true
    hosts:
      - host: synctv.yourdomain.com
        paths:
          - path: /
            pathType: Prefix
    tls:
      - secretName: synctv-tls
        hosts:
          - synctv.yourdomain.com

# Database Configuration
config:
  database:
    host: "postgresql.synctv.svc.cluster.local"
    password: "your-secure-password"

  redis:
    host: "redis-master.synctv.svc.cluster.local"
    password: "your-redis-password"

# Secrets (IMPORTANT: Change in production!)
secrets:
  database:
    password: "your-secure-password"
  redis:
    password: "your-redis-password"
  jwt:
    secret: "your-256-bit-random-jwt-secret"
  cluster:
    grpcSecret: "your-cluster-secret"
  provider:
    authSecret: "your-provider-secret"
```

### 4. Install SyncTV

```bash
helm install synctv ./helm/synctv \
  --namespace synctv \
  --create-namespace \
  --values my-values.yaml
```

Or with remote chart:

```bash
helm install synctv synctv/synctv \
  --namespace synctv \
  --create-namespace \
  --values my-values.yaml
```

### 5. Verify Installation

```bash
# Check pods
kubectl get pods -n synctv

# Check services
kubectl get svc -n synctv

# Check ingress
kubectl get ingress -n synctv

# View logs
kubectl logs -n synctv -l app.kubernetes.io/component=api -f
```

## Configuration

### Core Components

#### API Deployment (Stateless)

```yaml
api:
  enabled: true
  replicaCount: 3
  resources:
    requests:
      cpu: 500m
      memory: 512Mi
    limits:
      cpu: 2000m
      memory: 2Gi
  autoscaling:
    enabled: true
    minReplicas: 3
    maxReplicas: 10
```

#### Streaming StatefulSet (Stateful)

```yaml
streaming:
  enabled: true
  replicaCount: 3
  persistence:
    enabled: true
    storageClass: "fast-ssd"
    size: 100Gi
  resources:
    requests:
      cpu: 1000m
      memory: 2Gi
    limits:
      cpu: 4000m
      memory: 8Gi
```

### Advanced Configuration

#### OAuth2 Providers

```yaml
config:
  oauth2:
    providers:
      - name: "google"
        clientId: "your-google-client-id"
        redirectUrl: "https://synctv.example.com/api/oauth2/callback/google"
      - name: "github"
        clientId: "your-github-client-id"
        redirectUrl: "https://synctv.example.com/api/oauth2/callback/github"

secrets:
  oauth2:
    google:
      clientSecret: "your-google-client-secret"
    github:
      clientSecret: "your-github-client-secret"
```

#### Email (SMTP)

```yaml
config:
  email:
    enabled: true
    host: "smtp.gmail.com"
    port: 587
    from: "noreply@synctv.example.com"

secrets:
  email:
    username: "your-email@gmail.com"
    password: "your-app-specific-password"
```

#### S3 Storage for HLS

```yaml
config:
  streaming:
    hls:
      storageType: "s3"
    storage:
      s3:
        region: "us-east-1"
        bucket: "synctv-streaming"
        endpoint: "" # optional, for S3-compatible services

# Add AWS credentials as extra env vars
extraEnvVars:
  - name: AWS_ACCESS_KEY_ID
    value: "your-access-key"
  - name: AWS_SECRET_ACCESS_KEY
    value: "your-secret-key"
```

#### Node Affinity for Streaming

```yaml
streaming:
  nodeSelector:
    workload: streaming
  tolerations:
    - key: "streaming"
      operator: "Equal"
      value: "true"
      effect: "NoSchedule"
```

Label your streaming nodes:

```bash
kubectl label nodes node-1 node-2 node-3 workload=streaming
kubectl taint nodes node-1 node-2 node-3 streaming=true:NoSchedule
```

### Security Best Practices

#### 1. Generate Secure Secrets

```bash
# JWT Secret (256-bit)
openssl rand -base64 32

# Generic secrets
openssl rand -hex 32
```

#### 2. Use External Secrets Operator

```yaml
# Install External Secrets Operator
helm repo add external-secrets https://charts.external-secrets.io
helm install external-secrets external-secrets/external-secrets \
  --namespace external-secrets-system \
  --create-namespace

# Configure in values.yaml
extraEnvVarsSecret: "synctv-external-secrets"
```

#### 3. Enable Network Policies

```yaml
networkPolicy:
  enabled: true
  policyTypes:
    - Ingress
    - Egress
```

#### 4. Use TLS for Ingress

```yaml
api:
  ingress:
    annotations:
      cert-manager.io/cluster-issuer: "letsencrypt-prod"
    tls:
      - secretName: synctv-tls
        hosts:
          - synctv.example.com
```

## Upgrading

### Upgrade Helm Release

```bash
helm upgrade synctv ./helm/synctv \
  --namespace synctv \
  --values my-values.yaml
```

### Rollback

```bash
# View history
helm history synctv -n synctv

# Rollback to previous version
helm rollback synctv -n synctv

# Rollback to specific revision
helm rollback synctv 3 -n synctv
```

## Uninstallation

```bash
# Uninstall SyncTV
helm uninstall synctv -n synctv

# Optionally delete PVCs (WARNING: This deletes streaming data!)
kubectl delete pvc -n synctv -l app.kubernetes.io/instance=synctv

# Delete namespace
kubectl delete namespace synctv
```

## Troubleshooting

### Check Pod Status

```bash
kubectl get pods -n synctv
kubectl describe pod <pod-name> -n synctv
kubectl logs <pod-name> -n synctv -f
```

### Common Issues

#### 1. Pods Stuck in Pending

```bash
kubectl describe pod <pod-name> -n synctv

# Common causes:
# - Insufficient resources
# - PVC not bound (check: kubectl get pvc -n synctv)
# - Node selector mismatch
```

#### 2. Database Connection Failed

```bash
# Test PostgreSQL connection
kubectl run -it --rm debug --image=postgres:14 --restart=Never -n synctv -- \
  psql -h postgresql.synctv.svc.cluster.local -U synctv -d synctv

# Check secret
kubectl get secret synctv-secrets -n synctv -o yaml
```

#### 3. Ingress Not Working

```bash
# Check ingress
kubectl get ingress -n synctv
kubectl describe ingress synctv -n synctv

# Check ingress controller logs
kubectl logs -n ingress-nginx -l app.kubernetes.io/component=controller -f
```

#### 4. Streaming Not Accessible

```bash
# Check LoadBalancer service
kubectl get svc synctv-streaming -n synctv

# Get external IP
export STREAMING_IP=$(kubectl get svc synctv-streaming -n synctv -o jsonpath='{.status.loadBalancer.ingress[0].ip}')

# Test RTMP (requires ffmpeg)
ffmpeg -i input.mp4 -c copy -f flv rtmp://$STREAMING_IP:1935/live/stream
```

## Monitoring

### Prometheus Integration

```yaml
metrics:
  enabled: true
  serviceMonitor:
    enabled: true
    namespace: monitoring
    interval: 30s
    labels:
      prometheus: kube-prometheus
```

### View Metrics

```bash
# Port-forward to API pod
kubectl port-forward -n synctv svc/synctv-api 8080:8080

# Access metrics
curl http://localhost:8080/metrics
```

## Production Checklist

- [ ] Change all default secrets in `secrets` section
- [ ] Configure external PostgreSQL and Redis
- [ ] Enable TLS for ingress (cert-manager)
- [ ] Configure OAuth2 providers (if needed)
- [ ] Set up SMTP for email notifications
- [ ] Configure appropriate resource limits
- [ ] Enable autoscaling for API
- [ ] Set up pod disruption budgets
- [ ] Configure node affinity for streaming nodes
- [ ] Enable network policies
- [ ] Set up monitoring (Prometheus/Grafana)
- [ ] Configure backup for PostgreSQL
- [ ] Test disaster recovery procedures
- [ ] Set up log aggregation (ELK/Loki)
- [ ] Configure S3 for HLS storage (optional)
- [ ] Review and adjust rate limits

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Ingress (HTTPS)                        │
│                  synctv.example.com                         │
└───────────────┬─────────────────────────────────────────────┘
                │
    ┌───────────▼────────────┐
    │   API Deployment       │
    │   (3+ replicas)        │
    │   HTTP: 8080           │
    │   gRPC: 50051          │
    └───────────┬────────────┘
                │
    ┌───────────▼────────────┐         ┌─────────────────────┐
    │  Streaming StatefulSet │◄────────┤  LoadBalancer       │
    │  (3+ replicas)         │         │  RTMP: 1935         │
    │  RTMP: 1935            │         │  HLS: 8081          │
    │  HLS: 8081             │         └─────────────────────┘
    │  HTTP-FLV: 8082        │
    └───────────┬────────────┘
                │
        ┌───────┴────────┐
        │                │
   ┌────▼─────┐    ┌────▼─────┐
   │PostgreSQL│    │  Redis   │
   │ (External)│    │(External)│
   └──────────┘    └──────────┘
```

## Support

- GitHub: https://github.com/synctv-org/synctv
- Issues: https://github.com/synctv-org/synctv/issues
- Documentation: https://docs.synctv.example.com

## License

MIT License - see LICENSE file for details.
