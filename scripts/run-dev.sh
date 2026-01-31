#!/bin/bash
# Run SyncTV services in development mode

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Load .env if it exists
if [ -f "$PROJECT_ROOT/.env" ]; then
    export $(grep -v '^#' "$PROJECT_ROOT/.env" | xargs)
fi

# Default to running both services
RUN_API=${RUN_API:-true}
RUN_STREAM=${RUN_STREAM:-true}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --api-only)
            RUN_STREAM=false
            shift
            ;;
        --stream-only)
            RUN_API=false
            shift
            ;;
        --help)
            echo "Usage: $0 [--api-only|--stream-only]"
            echo ""
            echo "Options:"
            echo "  --api-only     Run only the API server"
            echo "  --stream-only  Run only the stream server"
            echo "  --help         Show this help message"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            echo "Run '$0 --help' for usage information"
            exit 1
            ;;
    esac
done

cd "$PROJECT_ROOT"

echo "🚀 Starting SyncTV services in development mode..."
echo ""

# Check if Docker services are running
if ! docker-compose ps postgres | grep -q "Up"; then
    echo "⚠️  PostgreSQL is not running. Starting with docker-compose up -d..."
    docker-compose up -d postgres redis
    sleep 3
fi

# Function to cleanup on exit
cleanup() {
    echo ""
    echo "🛑 Stopping services..."
    jobs -p | xargs -r kill 2>/dev/null
    exit 0
}

trap cleanup SIGINT SIGTERM

# Start API server
if [ "$RUN_API" = true ]; then
    echo "Starting synctv-api on http://localhost:${API_PORT:-8080}..."
    cargo run --bin synctv-api &
    API_PID=$!
fi

# Start stream server
if [ "$RUN_STREAM" = true ]; then
    echo "Starting synctv-stream (RTMP: ${RTMP_ADDR:-0.0.0.0:1935}, gRPC: ${GRPC_ADDR:-0.0.0.0:50052})..."
    cargo run --bin synctv-stream &
    STREAM_PID=$!
fi

echo ""
echo "✅ Services started!"
echo ""
if [ "$RUN_API" = true ]; then
    echo "API Server:"
    echo "  HTTP: http://localhost:${API_PORT:-8080}"
    echo "  gRPC: localhost:${GRPC_PORT:-50051}"
    echo "  Docs: http://localhost:${API_PORT:-8080}/api/docs"
fi
if [ "$RUN_STREAM" = true ]; then
    echo ""
    echo "Stream Server:"
    echo "  RTMP: rtmp://localhost:${RTMP_PORT:-1935}"
    echo "  gRPC: localhost:${GRPC_ADDR##*:}"
fi
echo ""
echo "Press Ctrl+C to stop all services"
echo ""

# Wait for all background jobs
wait
