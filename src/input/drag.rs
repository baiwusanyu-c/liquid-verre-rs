//! 拖拽状态机（方案 6.2 / 12.2）：
//! - 鼠标移动只记 pending，每帧应用一次（1000Hz -> 144Hz 合并）；
//! - 速度外推补偿采集滞后。

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DragKind {
    Move,
    ResizeSE,
}

pub struct DragState {
    pub id: u32,
    pub kind: DragKind,
    pub start_cursor: [f32; 2],
    pub origin: [f32; 4], // x, y, w, h
    pub velocity: [f32; 2],
    last_cursor: [f32; 2],
    last_time: f64,
    pub active: bool,
}

impl DragState {
    pub fn begin(id: u32, kind: DragKind, cursor: [f32; 2], origin: [f32; 4], now: f64) -> Self {
        Self {
            id,
            kind,
            start_cursor: cursor,
            origin,
            velocity: [0.0, 0.0],
            last_cursor: cursor,
            last_time: now,
            active: true,
        }
    }

    /// 每帧应用一次：返回 (x, y, w, h)
    pub fn apply(&mut self, cursor: [f32; 2], now: f64) -> [f32; 4] {
        let dt = (now - self.last_time).max(1e-4) as f32;
        let inst = [
            (cursor[0] - self.last_cursor[0]) / dt,
            (cursor[1] - self.last_cursor[1]) / dt,
        ];
        // 指数平滑，避免速度噪声导致画面抖动
        self.velocity[0] = self.velocity[0] * 0.75 + inst[0] * 0.25;
        self.velocity[1] = self.velocity[1] * 0.75 + inst[1] * 0.25;
        self.last_cursor = cursor;
        self.last_time = now;

        let dx = cursor[0] - self.start_cursor[0];
        let dy = cursor[1] - self.start_cursor[1];
        match self.kind {
            DragKind::Move => [
                self.origin[0] + dx,
                self.origin[1] + dy,
                self.origin[2],
                self.origin[3],
            ],
            DragKind::ResizeSE => [
                self.origin[0],
                self.origin[1],
                (self.origin[2] + dx).max(crate::glass::block::MIN_SIZE),
                (self.origin[3] + dy).max(crate::glass::block::MIN_SIZE),
            ],
        }
    }

    pub fn end(&mut self) {
        self.active = false;
        self.velocity = [0.0, 0.0];
    }
}
