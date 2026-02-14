use {std::collections::VecDeque, std::sync::Arc, crate::streamhub::define::FrameData};

/// Max frames per GOP to prevent unbounded memory growth.
/// 1500 frames ≈ 1 minute at 24fps, generous for any reasonable GOP.
const MAX_FRAMES_PER_GOP: usize = 1500;

/// A single Group of Pictures.
///
/// Internally stores frames in `Arc<Vec<FrameData>>` so that cloning a `Gop`
/// (e.g., when a new subscriber joins and receives cached GOPs) is O(1) --
/// only the Arc reference count is bumped, not the entire frame payload.
///
/// While the GOP is still being built (active GOP at the back of the deque),
/// frames are accumulated in `pending`. When the GOP is finalized (next keyframe
/// arrives) or when `get_gops()` is called, pending frames are frozen into
/// the Arc.
#[derive(Clone)]
pub struct Gop {
    /// Frozen (immutable) frames -- cheap to clone via Arc.
    frozen: Arc<Vec<FrameData>>,
    /// Frames being accumulated for the current (active) GOP.
    /// Empty once frozen.
    pending: Vec<FrameData>,
}

impl Default for Gop {
    fn default() -> Self {
        Self::new()
    }
}

impl Gop {
    #[must_use]
    pub fn new() -> Self {
        Self {
            frozen: Arc::new(Vec::new()),
            pending: Vec::new(),
        }
    }

    fn save_frame_data(&mut self, data: FrameData) {
        let total = self.frozen.len() + self.pending.len();
        if total >= MAX_FRAMES_PER_GOP {
            if total == MAX_FRAMES_PER_GOP {
                tracing::warn!(
                    "GOP reached MAX_FRAMES_PER_GOP ({MAX_FRAMES_PER_GOP}), dropping subsequent frames until next keyframe"
                );
            }
            return;
        }
        self.pending.push(data);
    }

    /// Freeze pending frames into the Arc for zero-copy clone.
    fn freeze(&mut self) {
        if !self.pending.is_empty() {
            let mut all_frames = Vec::with_capacity(self.frozen.len() + self.pending.len());
            all_frames.extend_from_slice(&self.frozen);
            all_frames.append(&mut self.pending);
            self.frozen = Arc::new(all_frames);
        }
    }

    /// Get all frame data (frozen + pending).
    #[must_use]
    pub fn get_frame_data(mut self) -> Vec<FrameData> {
        self.freeze();
        Arc::try_unwrap(self.frozen).unwrap_or_else(|arc| (*arc).clone())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.frozen.len() + self.pending.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone)]
pub struct Gops {
    gops: VecDeque<Gop>,
    size: usize,
}

impl Default for Gops {
    fn default() -> Self {
        Self::new(1)
    }
}

impl Gops {
    #[must_use]
    pub fn new(size: usize) -> Self {
        Self {
            gops: VecDeque::from([Gop::new()]),
            size,
        }
    }

    pub fn save_frame_data(&mut self, data: FrameData, is_key_frame: bool) {
        if self.size == 0 {
            return;
        }

        if is_key_frame {
            // Freeze the current back GOP before pushing a new one,
            // so it's ready for zero-copy clone.
            if let Some(back) = self.gops.back_mut() {
                back.freeze();
            }
            if self.gops.len() == self.size {
                self.gops.pop_front();
            }
            self.gops.push_back(Gop::new());
        }

        if let Some(gop) = self.gops.back_mut() {
            gop.save_frame_data(data);
        } else {
            tracing::error!("should not be here!");
        }
    }

    #[must_use]
    pub const fn setted(&self) -> bool {
        self.size != 0
    }

    /// Get all GOPs. Freezes any pending frames first for zero-copy clone.
    #[must_use]
    pub fn get_gops(&mut self) -> VecDeque<Gop> {
        // Freeze the active GOP so clone is cheap
        if let Some(back) = self.gops.back_mut() {
            back.freeze();
        }
        self.gops.clone()
    }
}
