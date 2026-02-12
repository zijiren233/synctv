use {std::collections::VecDeque, crate::streamhub::define::FrameData};

/// Max frames per GOP to prevent unbounded memory growth.
/// 1500 frames ≈ 1 minute at 24fps, generous for any reasonable GOP.
const MAX_FRAMES_PER_GOP: usize = 1500;

#[derive(Clone)]
pub struct Gop {
    datas: Vec<FrameData>,
}

impl Default for Gop {
    fn default() -> Self {
        Self::new()
    }
}

impl Gop {
    #[must_use] 
    pub const fn new() -> Self {
        Self { datas: Vec::new() }
    }

    fn save_frame_data(&mut self, data: FrameData) {
        if self.datas.len() >= MAX_FRAMES_PER_GOP {
            return;
        }
        self.datas.push(data);
    }

    #[must_use] 
    pub fn get_frame_data(self) -> Vec<FrameData> {
        self.datas
    }

    #[must_use] 
    pub const fn len(&self) -> usize {
        self.datas.len()
    }

    #[must_use] 
    pub const fn is_empty(&self) -> bool {
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
            //todo It may be possible to optimize here
            if self.gops.len() == self.size {
                self.gops.pop_front();
            }
            self.gops.push_back(Gop::new());
        }

        if let Some(gop) = self.gops.back_mut() {
            gop.save_frame_data(data);
        } else {
            log::error!("should not be here!");
        }
    }

    #[must_use] 
    pub const fn setted(&self) -> bool {
        self.size != 0
    }

    #[must_use] 
    pub fn get_gops(&self) -> VecDeque<Gop> {
        self.gops.clone()
    }
}
