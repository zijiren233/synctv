# Secrets Management Guide

This guide explains how to securely manage secrets in SyncTV production deployments.

## Overview

SyncTV supports multiple methods for managing sensitive configuration:
- **File-based secrets** (recommended for Kubernetes/Docker)
- **Environment variables** (fallback, less secure)
- **Optional secrets** (for features that can be disabled)

## Security Principles

### ✅ DO
- Store secrets in files mounted from secure secret stores
- Use Kubernetes Secrets or Docker secrets
- Restrict file permissions (`chmod 400` or `600`)
- Rotate secrets regularly
- Use different secrets for each environment
- Enable audit logging for secret access

### ❌ DON'T
- Commit secrets to Git
- Include secrets in Docker images
- Pass secrets as command-line arguments
- Log secret values
- Share secrets between environments
- Store secrets in environment variables (visible in `/proc/<pid>/environ`)

## Configuration Methods

### 1. Kubernetes Secrets (Recommended)

**Create secrets:**
```bash
# Create secret from file
kubectl create secret generic synctv-secrets \
  --from-file=database-password=./db-password.txt \
  --from-file=jwt-private-key=./jwt_private.pem \
  --from-file=smtp-password=./smtp-password.txt \
  --from-file=redis-password=./redis-password.txt

# Create secret from literal values (less secure, visible in kubectl history)
kubectl create secret generic synctv-secrets \
  --from-literal=database-password='your-db-password' \
  --from-literal=smtp-password='your-smtp-password'
```

**Mount secrets in deployment:**
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: synctv
spec:
  template:
    spec:
      containers:
      - name: synctv
        image: synctv:latest
        volumeMounts:
        - name: secrets
          mountPath: /run/secrets
          readOnly: true
        env:
        - name: DATABASE_URL
          value: "postgresql://synctv@postgres:5432/synctv"
        - name: DATABASE_PASSWORD_FILE
          value: "/run/secrets/database-password"
        - name: JWT_PRIVATE_KEY_PATH
          value: "/run/secrets/jwt-private-key"
        - name: SMTP_PASSWORD_FILE
          value: "/run/secrets/smtp-password"
      volumes:
      - name: secrets
        secret:
          secretName: synctv-secrets
          defaultMode: 0400  # Read-only for owner
```

### 2. Docker Secrets

**Create secrets:**
```bash
# Create Docker secret from file
docker secret create synctv_db_password ./db-password.txt
docker secret create synctv_jwt_private_key ./jwt_private.pem
docker secret create synctv_smtp_password ./smtp-password.txt
```

**Use in Docker Compose:**
```yaml
version: '3.8'

services:
  synctv:
    image: synctv:latest
    secrets:
      - database_password
      - jwt_private_key
      - smtp_password
    environment:
      DATABASE_URL: postgresql://synctv@postgres:5432/synctv
      DATABASE_PASSWORD_FILE: /run/secrets/database_password
      JWT_PRIVATE_KEY_PATH: /run/secrets/jwt_private_key
      SMTP_PASSWORD_FILE: /run/secrets/smtp_password

secrets:
  database_password:
    external: true
    name: synctv_db_password
  jwt_private_key:
    external: true
    name: synctv_jwt_private_key
  smtp_password:
    external: true
    name: synctv_smtp_password
```

### 3. File-based Secrets (Manual)

For manual deployments or testing:

```bash
# Create secrets directory
mkdir -p /etc/synctv/secrets
chmod 700 /etc/synctv/secrets

# Create secret files
echo -n 'your-db-password' > /etc/synctv/secrets/database-password
echo -n 'your-smtp-password' > /etc/synctv/secrets/smtp-password

# Set proper permissions
chmod 400 /etc/synctv/secrets/*
chown synctv:synctv /etc/synctv/secrets/*
```

**Configuration:**
```bash
# Environment variables point to secret files
export DATABASE_PASSWORD_FILE=/etc/synctv/secrets/database-password
export SMTP_PASSWORD_FILE=/etc/synctv/secrets/smtp-password
export JWT_PRIVATE_KEY_PATH=/etc/synctv/secrets/jwt-private-key
```

### 4. Environment Variables (Development Only)

⚠️ **Not recommended for production** - visible in process list and container inspect.

```bash
# Development/testing only
export DATABASE_PASSWORD=dev-password
export SMTP_PASSWORD=dev-smtp-password
```

## Required Secrets

### Database Password
- **File path**: `/run/secrets/database-password` or custom
- **Env var**: `DATABASE_PASSWORD_FILE` (path) or `DATABASE_PASSWORD` (value)
- **Required**: Yes
- **Format**: Plain text password
- **Rotation**: Every 90 days recommended

### JWT Private Key
- **File path**: `/run/secrets/jwt-private-key` or custom via `JWT_PRIVATE_KEY_PATH`
- **Required**: Yes
- **Format**: RSA private key in PEM format (2048-bit minimum)
- **Rotation**: Every 180 days recommended
- **Generation**:
  ```bash
  openssl genrsa -out jwt_private.pem 2048
  openssl rsa -in jwt_private.pem -pubout -out jwt_public.pem
  ```

### SMTP Password
- **File path**: `/run/secrets/smtp-password` or custom
- **Env var**: `SMTP_PASSWORD_FILE` (path) or `SMTP_PASSWORD` (value)
- **Required**: Only if email features are enabled
- **Format**: Plain text password
- **Rotation**: Per email provider policy

### Redis Password (if Redis requires auth)
- **File path**: `/run/secrets/redis-password` or custom
- **Env var**: `REDIS_PASSWORD_FILE` (path) or `REDIS_PASSWORD` (value)
- **Required**: Only if Redis authentication is enabled
- **Format**: Plain text password
- **Rotation**: Every 90 days recommended

### OAuth2 Client Secrets
- **File path**: `/run/secrets/oauth2-<provider>-secret` or via config
- **Required**: Only if OAuth2 providers are configured
- **Format**: Plain text secret from OAuth2 provider
- **Rotation**: Per OAuth2 provider policy

## Configuration Update

SyncTV configuration can reference secrets via `_FILE` suffix:

**Original config.yml:**
```yaml
database:
  url: "postgresql://synctv:CHANGEME@postgres:5432/synctv"

email:
  smtp_password: "CHANGEME"
```

**Secure config.yml:**
```yaml
database:
  url: "postgresql://synctv@postgres:5432/synctv"  # Password loaded from file

email:
  smtp_password: ""  # Password loaded from file
```

**Environment variables:**
```bash
DATABASE_PASSWORD_FILE=/run/secrets/database-password
SMTP_PASSWORD_FILE=/run/secrets/smtp-password
```

## Secret Rotation

### Manual Rotation Process

1. **Generate new secret:**
   ```bash
   openssl rand -base64 32 > new-password.txt
   ```

2. **Update Kubernetes/Docker secret:**
   ```bash
   # Kubernetes
   kubectl create secret generic synctv-secrets-new \
     --from-file=database-password=./new-password.txt \
     --dry-run=client -o yaml | kubectl apply -f -

   # Docker
   docker secret create synctv_db_password_v2 ./new-password.txt
   ```

3. **Update deployment to use new secret**

4. **Wait for all pods/containers to restart**

5. **Update database with new password**

6. **Verify application functionality**

7. **Delete old secret:**
   ```bash
   kubectl delete secret synctv-secrets-old
   ```

### JWT Key Rotation

JWT key rotation requires special handling to avoid invalidating active sessions:

1. Add new public key to verification keys
2. Start signing new tokens with new private key
3. Keep old public key for verification (grace period: 30 days)
4. Remove old public key after grace period

## Verification

### Check Secret Loading

```bash
# Verify secrets are loaded (will show masked values)
kubectl logs -f deployment/synctv | grep "secret"

# Expected output:
# INFO Loading secret from file secret_name="database_password" source="file" path="/run/secrets/database-password"
# INFO Secret loaded successfully secret_name="database_password" secret_len=32
```

### Security Audit

```bash
# Check file permissions
kubectl exec deployment/synctv -- ls -la /run/secrets/

# Expected: -r-------- 1 root root (400 permissions)

# Verify secrets are not in environment
kubectl exec deployment/synctv -- env | grep -i password

# Expected: No matches for actual passwords, only *_FILE paths

# Check process list doesn't expose secrets
kubectl exec deployment/synctv -- ps aux | grep synctv

# Expected: No secret values in command line
```

## Troubleshooting

### "Failed to read secret from file"

**Cause**: File doesn't exist or no read permission

**Solution**:
```bash
# Check if secret exists
kubectl get secret synctv-secrets -o yaml

# Check pod mounts
kubectl describe pod <pod-name> | grep -A 10 Mounts

# Check file in container
kubectl exec <pod-name> -- ls -la /run/secrets/
```

### "Secret is empty"

**Cause**: Secret file exists but contains no data

**Solution**:
```bash
# Check secret content
kubectl get secret synctv-secrets -o jsonpath='{.data.database-password}' | base64 -d

# Recreate secret with proper content
```

### "Missing required secrets"

**Cause**: Application startup validation failed

**Solution**:
1. Check application logs for specific missing secrets
2. Ensure all required secrets are created
3. Verify secret names match configuration

## External Secret Management

For production environments, consider using dedicated secret management solutions:

### HashiCorp Vault

```bash
# Store secrets in Vault
vault kv put secret/synctv/prod \
  database_password="..." \
  smtp_password="..." \
  jwt_private_key=@jwt_private.pem

# Use Vault Agent or CSI driver to inject secrets
```

### AWS Secrets Manager

```bash
# Store secrets in AWS
aws secretsmanager create-secret \
  --name synctv/prod/database-password \
  --secret-string "your-password"

# Use External Secrets Operator or AWS Secrets CSI driver
```

### Azure Key Vault

```bash
# Store secrets in Azure
az keyvault secret set \
  --vault-name synctv-vault \
  --name database-password \
  --value "your-password"

# Use Secrets Store CSI Driver
```

## Compliance

### Audit Logging

Enable audit logging for secret access:

```yaml
# Kubernetes audit policy
apiVersion: audit.k8s.io/v1
kind: Policy
rules:
- level: RequestResponse
  resources:
  - group: ""
    resources: ["secrets"]
  verbs: ["get", "list", "watch"]
```

### Encryption at Rest

Ensure secrets are encrypted at rest:

```bash
# Kubernetes - verify encryption
kubectl get secret synctv-secrets -o yaml

# Should show encrypted data, not base64-encoded plaintext
```

## References

- [Kubernetes Secrets](https://kubernetes.io/docs/concepts/configuration/secret/)
- [Docker Secrets](https://docs.docker.com/engine/swarm/secrets/)
- [OWASP Secrets Management Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Secrets_Management_Cheat_Sheet.html)
- [CIS Docker Benchmark](https://www.cisecurity.org/benchmark/docker)
