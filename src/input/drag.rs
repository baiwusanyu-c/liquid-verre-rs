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
    /// x, y 为虚拟屏物理像素；w, h 为**逻辑单位**（主屏物理像素），
    /// 这样跨屏拖拽 / 缩放时不会被按下那一刻的像素尺寸锁死（方案 12.2.5）。
    pub origin: [f32; 4],
    pub velocity: [f32; 2],
    last_cursor: [f32; 2],
    last_time: f64,
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
        }
    }

    /// 每帧应用一次：返回 (x, y, w_logical, h_logical)。
    /// `scale` = 块当前所在屏的 DPI 比例，用于把光标位移换算成逻辑单位。
    pub fn apply(&mut self, cursor: [f32; 2], now: f64, scale: f32) -> [f32; 4] {
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
            DragKind::ResizeSE => {
                let s = scale.max(1e-3);
                [
                    self.origin[0],
                    self.origin[1],
                    (self.origin[2] + dx / s).max(crate::glass::block::MIN_SIZE),
                    (self.origin[3] + dy / s).max(crate::glass::block::MIN_SIZE),
                ]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DragKind, DragState};

    #[test]
    fn move_returns_origin_plus_delta() {
        let mut d = DragState::begin(7, DragKind::Move, [100.0, 100.0], [10.0, 20.0, 300.0, 200.0], 0.0);
        let r = d.apply([140.0, 90.0], 0.016, 1.0);
        assert_eq!(r, [50.0, 10.0, 300.0, 200.0]);
    }
    #[test]
    fn move_ignores_scale() {
        let mut d = DragState::begin(7, DragKind::Move, [0.0, 0.0], [0.0, 0.0, 100.0, 100.0], 0.0);
        let r = d.apply([50.0, 50.0], 0.016, 2.0);
        assert_eq!(r, [50.0, 50.0, 100.0, 100.0]);
    }
    #[test]
    fn velocity_ema_converges_to_constant_speed() {
        let mut d = DragState::begin(1, DragKind::Move, [0.0, 0.0], [0.0, 0.0, 10.0, 10.0], 0.0);
        let mut t = 0.0;
        for i in 1..=200 {
            t = i as f64 * 0.008;
            d.apply([i as f32 * 8.0, 0.0], t, 1.0); // 1000 px/s
        }
        assert!((d.velocity[0] - 1000.0).abs() < 1.0, "vel={}", d.velocity[0]);
        assert!(d.velocity[1].abs() < 1e-3);
    }
    #[test]
    fn zero_dt_is_clamped_not_panics() {
        let mut d = DragState::begin(1, DragKind::Move, [0.0, 0.0], [0.0, 0.0, 10.0, 10.0], 1.0);
        let _ = d.apply([5.0, 5.0], 1.0, 1.0); // dt = 0 -> 内部 clamp 到 1e-4
        assert!(d.velocity[0].is_finite());
    }
}
