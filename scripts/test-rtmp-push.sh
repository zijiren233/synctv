#!/bin/bash
# Test RTMP push to SyncTV stream server using FFmpeg

set -e

RTMP_URL="${1:-rtmp://localhost:1935}"
ROOM_ID="${2:-test-room-123}"
TOKEN="${3:-test-token}"

# Check if ffmpeg is installed
if ! command -v ffmpeg &> /dev/null; then
    echo "❌ ffmpeg not found. Install with:"
    echo "  macOS:  brew install ffmpeg"
    echo "  Ubuntu: sudo apt-get install ffmpeg"
    exit 1
fi

echo "🎥 Testing RTMP push to SyncTV"
echo ""
echo "Stream URL: $RTMP_URL/$ROOM_ID?token=$TOKEN"
echo "Room ID:    $ROOM_ID"
echo "Token:      $TOKEN"
echo ""
echo "This will generate a test video pattern and push it via RTMP."
echo "Press Ctrl+C to stop streaming."
echo ""

# Generate test pattern and stream
ffmpeg \
    -re \
    -f lavfi \
    -i "testsrc=size=1280x720:rate=30" \
    -f lavfi \
    -i "sine=frequency=1000:sample_rate=44100" \
    -c:v libx264 \
    -preset veryfast \
    -tune zerolatency \
    -b:v 2000k \
    -maxrate 2000k \
    -bufsize 4000k \
    -pix_fmt yuv420p \
    -g 60 \
    -c:a aac \
    -b:a 128k \
    -ar 44100 \
    -f flv \
    "$RTMP_URL/$ROOM_ID?token=$TOKEN"
