# Stage 1: Build
FROM rust:slim as builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    protobuf-compiler \
    pkg-config && \
    rm -rf /var/lib/apt/lists/*

# Set working directory
WORKDIR /app

# Copy entire source tree
COPY . .

# Build with cache mounts for cargo registry, git deps, and target directory
# This provides fast incremental builds without complex dummy file tricks
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --release --bin synctv &&
    cp /app/target/release/synctv /app/synctv &&
    strip /app/synctv

# Stage 2: Runtime image
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Create synctv for running the application
RUN useradd -m -u 1000 synctv

# Create necessary directories
RUN mkdir -p /app /app/keys /app/config &&
    chown -R synctv:synctv /app

# Set working directory
WORKDIR /app

# Copy binary from builder
COPY --from=builder --chown=synctv:synctv /app/synctv /app/synctv

# Switch to non-root user
USER synctv

# Expose ports
# 8080: HTTP API
# 50051: gRPC API
EXPOSE 8080 50051

# Set environment variables
ENV RUST_LOG=info
ENV RUST_BACKTRACE=1

# Run the application
CMD ["/app/synctv"]
