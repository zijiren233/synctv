# Production-ready multi-stage Dockerfile for SyncTV
# This Dockerfile creates an optimized production image with:
# - Multi-stage build to minimize image size
# - Non-root user for security
# - Health checks
# - Minimal attack surface

# Stage 1: Build dependencies and compile
FROM rust:1.75-slim as builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    protobuf-compiler \
    libssl-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Create appuser for running the application
RUN useradd -m -u 1000 appuser

# Set working directory
WORKDIR /app

# Copy only dependency files first for better caching
COPY Cargo.toml Cargo.lock ./
COPY synctv/Cargo.toml synctv/
COPY synctv-api/Cargo.toml synctv-api/
COPY synctv-core/Cargo.toml synctv-core/
COPY synctv-cluster/Cargo.toml synctv-cluster/
COPY synctv-proto/Cargo.toml synctv-proto/
COPY synctv-providers/Cargo.toml synctv-providers/
COPY synctv-proxy/Cargo.toml synctv-proxy/
COPY synctv-sfu/Cargo.toml synctv-sfu/
COPY synctv-stream/Cargo.toml synctv-stream/
COPY synctv-migration/Cargo.toml synctv-migration/

# Create dummy source files to build dependencies
RUN mkdir -p synctv/src synctv-api/src synctv-core/src synctv-cluster/src \
    synctv-proto/src synctv-providers/src synctv-proxy/src synctv-sfu/src \
    synctv-stream/src synctv-migration/src && \
    echo "fn main() {}" > synctv/src/main.rs && \
    echo "fn main() {}" > synctv/src/bin/placeholder.rs && \
    find . -name Cargo.toml -not -path "./Cargo.toml" -exec sh -c 'echo "" > $(dirname {})/src/lib.rs' \;

# Build dependencies (this layer will be cached)
RUN cargo build --release

# Remove dummy files
RUN rm -rf synctv*/src

# Copy actual source code
COPY . .

# Touch files to force rebuild of our code only
RUN find . -name "*.rs" -exec touch {} \;

# Build the actual application
RUN cargo build --release --bin synctv

# Strip debug symbols to reduce binary size
RUN strip /app/target/release/synctv

# Stage 2: Runtime image
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create appuser for running the application
RUN useradd -m -u 1000 appuser

# Create necessary directories
RUN mkdir -p /app /app/keys /app/config && \
    chown -R appuser:appuser /app

# Set working directory
WORKDIR /app

# Copy binary from builder
COPY --from=builder --chown=appuser:appuser /app/target/release/synctv /app/synctv

# Switch to non-root user
USER appuser

# Expose ports
# 8080: HTTP API
# 50051: gRPC API
EXPOSE 8080 50051

# Health check using the /health/live endpoint
HEALTHCHECK --interval=30s --timeout=3s --start-period=40s --retries=3 \
    CMD ["/usr/bin/curl", "-f", "http://localhost:8080/health/live", "||", "exit", "1"]

# Set environment variables
ENV RUST_LOG=info
ENV RUST_BACKTRACE=1

# Run the application
CMD ["/app/synctv"]
