//! [未接入 / 当前不参与编译] WGC 后端的最新帧发布槽位，随 `wgc.rs` 一起启用。
//! 最新采集帧的发布槽位（方案 3.2）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

pub struct FrameSlot {
    frame: Mutex<Option<ID3D11Texture2D>>,
    pub width: AtomicU64,
    pub height: AtomicU64,
    pub frames: AtomicU64,
}

impl Default for FrameSlot {
    fn default() -> Self {
        Self {
            frame: Mutex::new(None),
            width: AtomicU64::new(0),
            height: AtomicU64::new(0),
            frames: AtomicU64::new(0),
        }
    }
}

impl FrameSlot {
    pub fn publish(&self, tex: ID3D11Texture2D, w: i32, h: i32) {
        *self.frame.lock().unwrap() = Some(tex);
        self.width.store(w as u64, Ordering::Relaxed);
        self.height.store(h as u64, Ordering::Relaxed);
        self.frames.fetch_add(1, Ordering::Relaxed);
    }

    pub fn take(&self) -> Option<(ID3D11Texture2D, i32, i32)> {
        let mut guard = self.frame.lock().unwrap();
        guard.take().map(|t| {
            let w = self.width.load(Ordering::Relaxed) as i32;
            let h = self.height.load(Ordering::Relaxed) as i32;
            (t, w, h)
        })
    }

    pub fn frame_count(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }
}
