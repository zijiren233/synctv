#!/bin/bash
# Setup development environment for SyncTV

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

echo "🚀 Setting up SyncTV development environment..."
echo ""

# Check required tools
echo "Checking required tools..."
command -v cargo >/dev/null 2>&1 || { echo "❌ cargo not found. Install Rust from https://rustup.rs/"; exit 1; }
command -v docker >/dev/null 2>&1 || { echo "❌ docker not found. Install Docker from https://docker.com/"; exit 1; }
command -v docker-compose >/dev/null 2>&1 || { echo "❌ docker-compose not found"; exit 1; }
command -v openssl >/dev/null 2>&1 || { echo "❌ openssl not found"; exit 1; }
command -v sqlx >/dev/null 2>&1 || { echo "⚠️  sqlx-cli not found. Installing..."; cargo install sqlx-cli --no-default-features --features postgres; }

echo "✓ All required tools found"
echo ""

# Generate JWT keys if they don't exist
KEYS_DIR="$PROJECT_ROOT/keys"
if [ ! -f "$KEYS_DIR/jwt_private.pem" ]; then
    echo "Generating JWT RSA keys..."
    "$SCRIPT_DIR/generate-jwt-keys.sh" "$KEYS_DIR"
    echo ""
else
    echo "✓ JWT keys already exist at $KEYS_DIR"
    echo ""
fi

# Start Docker services (PostgreSQL, Redis)
echo "Starting Docker services (PostgreSQL, Redis)..."
cd "$PROJECT_ROOT"

if [ -f "docker-compose.yml" ]; then
    docker-compose up -d postgres redis
    echo "✓ Docker services started"
    echo ""

    # Wait for PostgreSQL to be ready
    echo "Waiting for PostgreSQL to be ready..."
    for i in {1..30}; do
        if docker-compose exec -T postgres pg_isready -U synctv >/dev/null 2>&1; then
            echo "✓ PostgreSQL is ready"
            break
        fi
        if [ $i -eq 30 ]; then
            echo "❌ PostgreSQL failed to start within 30 seconds"
            exit 1
        fi
        sleep 1
    done
    echo ""

    # Run database migrations
    echo "Running database migrations..."
    export DATABASE_URL="postgresql://synctv:synctv@localhost:5432/synctv"
    cd "$PROJECT_ROOT"
    sqlx migrate run
    echo "✓ Database migrations completed"
    echo ""
else
    echo "⚠️  docker-compose.yml not found. Creating one..."
    cat > docker-compose.yml <<'EOF'
version: '3.8'

services:
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_USER: synctv
      POSTGRES_PASSWORD: synctv
      POSTGRES_DB: synctv
    ports:
      - "5432:5432"
    volumes:
      - postgres_data:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U synctv"]
      interval: 5s
      timeout: 5s
      retries: 5

  redis:
    image: redis:7-alpine
    ports:
      - "6379:6379"
    volumes:
      - redis_data:/data
    healthcheck:
      test: ["CMD", "redis-cli", "ping"]
      interval: 5s
      timeout: 3s
      retries: 5

volumes:
  postgres_data:
  redis_data:
EOF
    echo "✓ Created docker-compose.yml"
    docker-compose up -d
    echo ""
fi

# Create .env file if it doesn't exist
if [ ! -f "$PROJECT_ROOT/.env" ]; then
    echo "Creating .env file..."
    cat > "$PROJECT_ROOT/.env" <<EOF
# Database
DATABASE_URL=postgresql://synctv:synctv@localhost:5432/synctv

# Redis
REDIS_URL=redis://localhost:6379

# JWT
SYNCTV__JWT__PRIVATE_KEY_PATH=$KEYS_DIR/jwt_private.pem
SYNCTV__JWT__PUBLIC_KEY_PATH=$KEYS_DIR/jwt_public.pem
SYNCTV__JWT__ACCESS_TOKEN_EXPIRES=3600
SYNCTV__JWT__REFRESH_TOKEN_EXPIRES=2592000

# API Server
API_HOST=0.0.0.0
API_PORT=8080
GRPC_PORT=50051

# Stream Server
RTMP_ADDR=0.0.0.0:1935
GRPC_ADDR=0.0.0.0:50052
MAX_STREAMS=50
ENABLE_GOP_CACHE=true
MAX_GOPS=2
MAX_GOP_CACHE_SIZE_MB=100

# Logging
RUST_LOG=info,synctv=debug
EOF
    echo "✓ Created .env file"
    echo ""
else
    echo "✓ .env file already exists"
    echo ""
fi

echo "✅ Development environment setup complete!"
echo ""
echo "Next steps:"
echo "  1. Build the project:    cargo build"
echo "  2. Run tests:            cargo test"
echo "  3. Start API server:     cargo run --bin synctv-api"
echo "  4. Start stream server:  cargo run --bin synctv-stream"
echo ""
echo "Services running:"
echo "  PostgreSQL: localhost:5432 (user: synctv, password: synctv, db: synctv)"
echo "  Redis:      localhost:6379"
echo ""
echo "To stop services:  docker-compose down"
echo "To view logs:      docker-compose logs -f"
