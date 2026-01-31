# Multi-stage build for SyncTV Stream Server

# Stage 1: Builder
FROM rust:1.75-slim as builder

WORKDIR /build

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace manifest
COPY Cargo.toml Cargo.lock ./

# Copy all crates
COPY synctv-core ./synctv-core
COPY synctv-cluster ./synctv-cluster
COPY synctv-stream ./synctv-stream

# Build release binary
RUN cargo build --release --package synctv-stream

# Stage 2: Runtime
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy binary from builder
COPY --from=builder /build/target/release/synctv-stream /app/synctv-stream

# Create HLS storage directory
RUN mkdir -p /data/hls

# Set environment variables
ENV RUST_LOG=info
ENV RTMP_ADDR=0.0.0.0:1935
ENV HTTPFLV_ADDR=0.0.0.0:8080
ENV HLS_ADDR=0.0.0.0:8081
ENV HLS_STORAGE=/data/hls
ENV REDIS_URL=redis://localhost:6379

# Expose ports
EXPOSE 1935 8080 8081

# Run the server
ENTRYPOINT ["/app/synctv-stream"]
