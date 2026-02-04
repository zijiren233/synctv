use std::sync::Arc;
use std::collections::VecDeque;
use bytes::Bytes;
use parking_lot::RwLock;
use dashmap::DashMap;
use tracing::{debug, warn};

/// Configuration for GOP cache
#[derive(Debug, Clone)]
pub struct GopCacheConfig {
    /// Maximum number of GOPs to cache (default: 2)
    pub max_gops: usize,
    /// Maximum total cache size in bytes (default: 100MB)
    pub max_cache_size: usize,
    /// Whether GOP cache is enabled
    pub enabled: bool,
}

impl Default for GopCacheConfig {
    fn default() -> Self {
        Self {
            max_gops: 2,
            max_cache_size: 100 * 1024 * 1024, // 100MB
            enabled: true,
        }
    }
}

/// A frame in the GOP cache
#[derive(Debug, Clone)]
pub struct GopFrame {
    /// Frame data
    pub data: Bytes,
    /// Timestamp in milliseconds
    pub timestamp: u32,
    /// Whether this is a keyframe (IDR frame)
    pub is_keyframe: bool,
    /// Frame type (video, audio)
    pub frame_type: FrameType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    Video,
    Audio,
}

/// GOP (Group of Pictures) cache
/// Stores the last N GOPs for fast startup when new viewers join
pub struct GopCache {
    config: GopCacheConfig,
    /// Stream ID -> GOP cache
    caches: Arc<DashMap<String, Arc<RwLock<StreamGopCache>>>>,
}

/// Per-stream GOP cache
struct StreamGopCache {
    /// GOPs stored as a ring buffer
    gops: VecDeque<Vec<GopFrame>>,
    /// Current GOP being built
    current_gop: Vec<GopFrame>,
    /// Total size of cached data in bytes
    total_size: usize,
}

impl GopCache {
    /// Create a new GOP cache
    #[must_use] 
    pub fn new(config: GopCacheConfig) -> Self {
        Self {
            config,
            caches: Arc::new(DashMap::new()),
        }
    }

    /// Add a frame to the cache
    pub fn add_frame(&self, stream_id: &str, frame: GopFrame) {
        if !self.config.enabled {
            return;
        }

        let cache = self.caches
            .entry(stream_id.to_string())
            .or_insert_with(|| {
                Arc::new(RwLock::new(StreamGopCache {
                    gops: VecDeque::new(),
                    current_gop: Vec::new(),
                    total_size: 0,
                }))
            })
            .clone();

        let mut cache = cache.write();

        // If this is a keyframe, start a new GOP
        if frame.is_keyframe {
            // Move current GOP to completed GOPs
            if !cache.current_gop.is_empty() {
                let completed_gop = std::mem::take(&mut cache.current_gop);
                cache.gops.push_back(completed_gop);

                // Evict old GOPs if we exceed max_gops
                while cache.gops.len() > self.config.max_gops {
                    if let Some(old_gop) = cache.gops.pop_front() {
                        let old_size: usize = old_gop.iter()
                            .map(|f| f.data.len())
                            .sum();
                        cache.total_size = cache.total_size.saturating_sub(old_size);
                    }
                }
            }
        }

        // Add frame to current GOP
        let frame_size = frame.data.len();
        cache.current_gop.push(frame);
        cache.total_size += frame_size;

        // Check if we exceed max cache size
        if cache.total_size > self.config.max_cache_size {
            warn!(
                stream_id = stream_id,
                total_size = cache.total_size,
                max_size = self.config.max_cache_size,
                "GOP cache size exceeded, evicting oldest GOP"
            );

            // Evict oldest GOP
            if let Some(old_gop) = cache.gops.pop_front() {
                let old_size: usize = old_gop.iter()
                    .map(|f| f.data.len())
                    .sum();
                cache.total_size = cache.total_size.saturating_sub(old_size);
            }
        }

        debug!(
            stream_id = stream_id,
            gops = cache.gops.len(),
            current_gop_frames = cache.current_gop.len(),
            total_size = cache.total_size,
            "Frame added to GOP cache"
        );
    }

    /// Get all cached frames for a stream
    /// Returns frames from all complete GOPs plus the current GOP
    #[must_use] 
    pub fn get_frames(&self, stream_id: &str) -> Vec<GopFrame> {
        if !self.config.enabled {
            return Vec::new();
        }

        self.caches
            .get(stream_id)
            .map(|cache| {
                let cache = cache.read();
                let mut frames = Vec::new();

                // Add all complete GOPs
                for gop in &cache.gops {
                    frames.extend_from_slice(gop);
                }

                // Add current GOP
                frames.extend_from_slice(&cache.current_gop);

                frames
            })
            .unwrap_or_default()
    }

    /// Clear cache for a specific stream
    pub fn clear_stream(&self, stream_id: &str) {
        self.caches.remove(stream_id);
        debug!(stream_id = stream_id, "GOP cache cleared for stream");
    }

    /// Get cache statistics for a stream
    #[must_use] 
    pub fn get_stats(&self, stream_id: &str) -> Option<GopCacheStats> {
        self.caches.get(stream_id).map(|cache| {
            let cache = cache.read();
            GopCacheStats {
                gop_count: cache.gops.len(),
                current_gop_frames: cache.current_gop.len(),
                total_size: cache.total_size,
                total_frames: cache.gops.iter()
                    .map(std::vec::Vec::len)
                    .sum::<usize>() + cache.current_gop.len(),
            }
        })
    }

    /// Get total cache size across all streams
    #[must_use] 
    pub fn total_size(&self) -> usize {
        self.caches
            .iter()
            .map(|entry| entry.read().total_size)
            .sum()
    }

    /// Clear all caches
    pub fn clear_all(&self) {
        self.caches.clear();
        debug!("All GOP caches cleared");
    }
}

/// GOP cache statistics
#[derive(Debug, Clone)]
pub struct GopCacheStats {
    /// Number of complete GOPs cached
    pub gop_count: usize,
    /// Number of frames in current (incomplete) GOP
    pub current_gop_frames: usize,
    /// Total size in bytes
    pub total_size: usize,
    /// Total number of frames
    pub total_frames: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_frame(is_keyframe: bool, size: usize) -> GopFrame {
        GopFrame {
            data: Bytes::from(vec![0u8; size]),
            timestamp: 0,
            is_keyframe,
            frame_type: FrameType::Video,
        }
    }

    #[test]
    fn test_gop_cache_add_frames() {
        let config = GopCacheConfig {
            max_gops: 2,
            max_cache_size: 1024 * 1024,
            enabled: true,
        };
        let cache = GopCache::new(config);

        // Add keyframe (starts new GOP)
        cache.add_frame("stream1", create_test_frame(true, 100));

        // Add regular frames
        cache.add_frame("stream1", create_test_frame(false, 50));
        cache.add_frame("stream1", create_test_frame(false, 50));

        let stats = cache.get_stats("stream1").unwrap();
        assert_eq!(stats.current_gop_frames, 3);
        assert_eq!(stats.gop_count, 0); // No complete GOPs yet
        assert_eq!(stats.total_size, 200);
    }

    #[test]
    fn test_gop_cache_multiple_gops() {
        let config = GopCacheConfig {
            max_gops: 2,
            max_cache_size: 1024 * 1024,
            enabled: true,
        };
        let cache = GopCache::new(config);

        // First GOP
        cache.add_frame("stream1", create_test_frame(true, 100));
        cache.add_frame("stream1", create_test_frame(false, 50));

        // Second GOP (keyframe completes first GOP)
        cache.add_frame("stream1", create_test_frame(true, 100));
        cache.add_frame("stream1", create_test_frame(false, 50));

        let stats = cache.get_stats("stream1").unwrap();
        assert_eq!(stats.gop_count, 1); // One complete GOP
        assert_eq!(stats.current_gop_frames, 2); // Current GOP has 2 frames
        assert_eq!(stats.total_frames, 4);
    }

    #[test]
    fn test_gop_cache_eviction() {
        let config = GopCacheConfig {
            max_gops: 2,
            max_cache_size: 1024 * 1024,
            enabled: true,
        };
        let cache = GopCache::new(config);

        // Add 3 GOPs (should evict the first one)
        for _i in 0..3 {
            cache.add_frame("stream1", create_test_frame(true, 100));
            cache.add_frame("stream1", create_test_frame(false, 50));
        }

        // Start 4th GOP
        cache.add_frame("stream1", create_test_frame(true, 100));

        let stats = cache.get_stats("stream1").unwrap();
        assert_eq!(stats.gop_count, 2); // Only 2 GOPs kept (max_gops)
        assert_eq!(stats.current_gop_frames, 1);
    }

    #[test]
    fn test_gop_cache_get_frames() {
        let config = GopCacheConfig::default();
        let cache = GopCache::new(config);

        // Add frames
        cache.add_frame("stream1", create_test_frame(true, 100));
        cache.add_frame("stream1", create_test_frame(false, 50));
        cache.add_frame("stream1", create_test_frame(true, 100));

        let frames = cache.get_frames("stream1");
        assert_eq!(frames.len(), 3);
        assert!(frames[0].is_keyframe);
        assert!(!frames[1].is_keyframe);
        assert!(frames[2].is_keyframe);
    }

    #[test]
    fn test_gop_cache_clear() {
        let config = GopCacheConfig::default();
        let cache = GopCache::new(config);

        cache.add_frame("stream1", create_test_frame(true, 100));
        cache.add_frame("stream1", create_test_frame(false, 50));

        assert!(cache.get_stats("stream1").is_some());

        cache.clear_stream("stream1");
        assert!(cache.get_stats("stream1").is_none());
    }
}
