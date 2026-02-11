# synctv-xiu

Consolidated streaming library for SyncTV, providing RTMP, HLS, and HTTP-FLV protocol support with FLV/MPEG-TS container formats.

This crate is derived from [xiu](https://github.com/harlanc/xiu) by HarlanC, restructured from 9 separate crates into a single unified crate with the following modules:

- **bytesio** - Async byte I/O utilities built on tokio
- **h264** - H.264 (AVC) bitstream parser (SPS/PPS)
- **flv** - FLV container format (muxer, demuxer, AMF0)
- **mpegts** - MPEG-TS container format (PAT/PMT/PES)
- **streamhub** - Central event bus for stream distribution
- **storage** - Pluggable HLS segment storage (file, memory, OSS/S3)
- **rtmp** - RTMP protocol (handshake, chunking, sessions)
- **hls** - HLS protocol (RTMP-to-HLS remuxer, segment management, HTTP server)
- **httpflv** - HTTP-FLV streaming

## License

MIT - see the [original project](https://github.com/harlanc/xiu) for details.
