#!/bin/bash
# Generate RSA keys for JWT signing

set -e

# Default output directory
KEYS_DIR="${1:-./keys}"

# Create keys directory if it doesn't exist
mkdir -p "$KEYS_DIR"

echo "Generating JWT RSA keys in $KEYS_DIR..."

# Generate private key (2048 bits)
openssl genrsa -out "$KEYS_DIR/jwt_private.pem" 2048

# Generate public key from private key
openssl rsa -in "$KEYS_DIR/jwt_private.pem" -pubout -out "$KEYS_DIR/jwt_public.pem"

echo "✓ JWT keys generated successfully!"
echo "  Private key: $KEYS_DIR/jwt_private.pem"
echo "  Public key:  $KEYS_DIR/jwt_public.pem"
echo ""
echo "Set environment variables:"
echo "  export SYNCTV__JWT__PRIVATE_KEY_PATH=$KEYS_DIR/jwt_private.pem"
echo "  export SYNCTV__JWT__PUBLIC_KEY_PATH=$KEYS_DIR/jwt_public.pem"
