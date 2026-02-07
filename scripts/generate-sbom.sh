#!/usr/bin/env bash
# Generate Software Bill of Materials (SBOM) for the project
#
# This script generates a CycloneDX SBOM in JSON format, which includes:
# - All direct and transitive dependencies
# - License information
# - Vulnerability data (when available)
#
# Requirements: cargo-cyclonedx
# Install: cargo install cargo-cyclonedx

set -euo pipefail

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}Generating Software Bill of Materials (SBOM)...${NC}"

# Check if cargo-cyclonedx is installed
if ! command -v cargo-cyclonedx &> /dev/null; then
    echo "cargo-cyclonedx not found. Installing..."
    cargo install cargo-cyclonedx
fi

# Generate SBOM
OUTPUT_DIR="target/sbom"
mkdir -p "$OUTPUT_DIR"

echo "Generating CycloneDX SBOM..."
cargo cyclonedx \
    --format json \
    --output-cdx "${OUTPUT_DIR}/synctv-sbom.json" \
    --output-pattern "{name}-{version}-{target}-sbom.json"

echo -e "${GREEN}✓ SBOM generated successfully${NC}"
echo "Location: ${OUTPUT_DIR}/synctv-sbom.json"
echo ""
echo "To view the SBOM, you can:"
echo "  1. Upload to https://cyclonedx.org/tool-center/"
echo "  2. Use a tool like 'jq' to inspect: jq . ${OUTPUT_DIR}/synctv-sbom.json"
echo "  3. Integrate with vulnerability scanners (Grype, Trivy, etc.)"
