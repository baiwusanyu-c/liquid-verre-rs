//! 拖拽 / 缩放状态机（方案 6.2 / 12.2 + 需求 F2）。
//!
//! 共同约定：
//! - 鼠标移动只记 pending，**每帧应用一次**（1000 Hz 消息 → 144 Hz 合并）；
//! - 速度用 EMA(0.75/0.25) 平滑，供着色器做采集滞后外推（`lag_seconds`）；
//! - `origin` 的 `x, y` 是虚拟屏物理像素，`w, h` 是**逻辑单位**（主屏物理像素），
//!   这样跨屏拖拽 / 缩放不会被按下那一刻的像素尺寸锁死（方案 12.2.5）；
//! - 缩放时**锚定不动边**：夹到最小尺寸时固定边必须纹丝不动，否则会出现「左边滑走」。

use crate::core::block::MIN_SIZE;
use crate::core::handle::ResizeEdges;

/// 一次进行中的鼠标会话：移动或缩放。
#[derive(Debug)]
pub(crate) enum DragSession {
    /// 整块移动。
    Move(MoveState),
    /// 边缘 / 对角缩放（需求 F2）。
    Resize(ResizeState),
}

impl DragSession {
    /// 被操作的块 id。
    #[must_use]
    pub(crate) fn id(&self) -> u32 {
        match self {
            DragSession::Move(s) => s.id,
            DragSession::Resize(s) => s.id,
        }
    }

    /// 是否为缩放假话。
    #[must_use]
    pub(crate) fn is_resize(&self) -> bool {
        matches!(self, DragSession::Resize(_))
    }

    /// 缩放会话的边集合；移动会话返回 `None`。
    #[must_use]
    pub(crate) fn edges(&self) -> Option<ResizeEdges> {
        match self {
            DragSession::Resize(s) => Some(s.edges),
            DragSession::Move(_) => None,
        }
    }

    /// 平滑后的光标速度（物理像素 / 秒）。
    #[must_use]
    pub(crate) fn velocity(&self) -> [f32; 2] {
        match self {
            DragSession::Move(s) => s.velocity,
            DragSession::Resize(s) => s.velocity,
        }
    }

    /// 每帧应用一次，返回 `(x, y, w_logical, h_logical)`。
    ///
    /// `scale` = 块当前所在屏的 DPI 比例。移动会话**不**用 scale 换算尺寸
    /// （尺寸归 DPI 换算管，否则跨屏后会被按下那一刻的像素尺寸覆盖 —— 这是
    /// 跨屏回归测试抓到的真实缺陷）。
    pub(crate) fn apply(
        &mut self,
        cursor: [f32; 2],
        now: f64,
        scale: f32,
        min_size: [f32; 2],
    ) -> [f32; 4] {
        match self {
            DragSession::Move(s) => s.apply(cursor, now),
            DragSession::Resize(s) => s.apply(cursor, now, scale, min_size),
        }
    }
}

/// 速度 EMA 系数：0.75 旧值 + 0.25 瞬时值（与重构前一致，不要动 —— 手感调过）。
const VEL_KEEP: f32 = 0.75;
const VEL_NEW: f32 = 0.25;

/// `dt` 下限，防止两次采样时间相同导致速度爆炸。
const MIN_DT: f64 = 1e-4;

fn ema_velocity(prev: [f32; 2], last_cursor: [f32; 2], cursor: [f32; 2], dt: f32) -> [f32; 2] {
    let inst = [
        (cursor[0] - last_cursor[0]) / dt,
        (cursor[1] - last_cursor[1]) / dt,
    ];
    [
        prev[0] * VEL_KEEP + inst[0] * VEL_NEW,
        prev[1] * VEL_KEEP + inst[1] * VEL_NEW,
    ]
}

/// 移动会话。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MoveState {
    pub id: u32,
    pub start_cursor: [f32; 2],
    /// `[x, y, w_logical, h_logical]`
    pub origin: [f32; 4],
    pub velocity: [f32; 2],
    last_cursor: [f32; 2],
    last_time: f64,
}

impl MoveState {
    pub(crate) fn begin(id: u32, cursor: [f32; 2], origin: [f32; 4], now: f64) -> Self {
        Self {
            id,
            start_cursor: cursor,
            origin,
            velocity: [0.0; 2],
            last_cursor: cursor,
            last_time: now,
        }
    }

    pub(crate) fn apply(&mut self, cursor: [f32; 2], now: f64) -> [f32; 4] {
        let dt = (now - self.last_time).max(MIN_DT) as f32;
        self.velocity = ema_velocity(self.velocity, self.last_cursor, cursor, dt);
        self.last_cursor = cursor;
        self.last_time = now;
        [
            self.origin[0] + (cursor[0] - self.start_cursor[0]),
            self.origin[1] + (cursor[1] - self.start_cursor[1]),
            self.origin[2],
            self.origin[3],
        ]
    }
}

/// 缩放会话（需求 F2）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ResizeState {
    pub id: u32,
    pub edges: ResizeEdges,
    pub start_cursor: [f32; 2],
    /// `[x, y, w_logical, h_logical]`
    pub origin: [f32; 4],
    pub velocity: [f32; 2],
    last_cursor: [f32; 2],
    last_time: f64,
}

impl ResizeState {
    pub(crate) fn begin(
        id: u32,
        edges: ResizeEdges,
        cursor: [f32; 2],
        origin: [f32; 4],
        now: f64,
    ) -> Self {
        Self {
            id,
            edges,
            start_cursor: cursor,
            origin,
            velocity: [0.0; 2],
            last_cursor: cursor,
            last_time: now,
        }
    }

    /// 返回 `(x, y, w_logical, h_logical)`。
    ///
    /// 锚定规则：
    /// - 拖右边 → 左边不动，`w = max(min_w, w0 + dx/s)`；
    /// - 拖左边 → 右边不动，`w = max(min_w, w0 - dx/s)`，且 `x = x0 + (w0 - w)`
    ///   （用 `w0 - w` 而不是 `dx/s`，这样夹到 `min_w` 时右边**精确**不动）；
    /// - 上下同理；对角 = 两个方向的组合。
    pub(crate) fn apply(
        &mut self,
        cursor: [f32; 2],
        now: f64,
        scale: f32,
        min_size: [f32; 2],
    ) -> [f32; 4] {
        let dt = (now - self.last_time).max(MIN_DT) as f32;
        self.velocity = ema_velocity(self.velocity, self.last_cursor, cursor, dt);
        self.last_cursor = cursor;
        self.last_time = now;

        let s = if scale.is_finite() && scale > 1e-3 {
            scale
        } else {
            1.0
        };
        let dx = (cursor[0] - self.start_cursor[0]) / s;
        let dy = (cursor[1] - self.start_cursor[1]) / s;
        let [x0, y0, w0, h0] = self.origin;
        let min_w = min_size[0].max(MIN_SIZE);
        let min_h = min_size[1].max(MIN_SIZE);

        let mut x = x0;
        let mut y = y0;
        let mut w = w0;
        let mut h = h0;

        if self.edges.right {
            w = (w0 + dx).max(min_w);
        } else if self.edges.left {
            w = (w0 - dx).max(min_w);
            x = x0 + (w0 - w);
        }
        if self.edges.bottom {
            h = (h0 + dy).max(min_h);
        } else if self.edges.top {
            h = (h0 - dy).max(min_h);
            y = y0 + (h0 - h);
        }
        [x, y, w, h]
    }
}

#[cfg(test)]
mod tests {
    use super::{DragSession, MoveState, ResizeState};
    use crate::core::handle::ResizeEdges as E;

    fn move_session() -> DragSession {
        DragSession::Move(MoveState::begin(
            7,
            [100.0, 100.0],
            [10.0, 20.0, 300.0, 200.0],
            0.0,
        ))
    }

    fn resize_session(edges: E) -> DragSession {
        DragSession::Resize(ResizeState::begin(
            7,
            edges,
            [0.0, 0.0],
            [100.0, 100.0, 300.0, 200.0],
            0.0,
        ))
    }

    #[test]
    fn move_returns_origin_plus_delta() {
        let mut d = move_session();
        let r = d.apply([140.0, 90.0], 0.016, 1.0, [24.0, 24.0]);
        assert_eq!(r, [50.0, 10.0, 300.0, 200.0]);
    }

    #[test]
    fn move_ignores_scale_for_size() {
        let mut d = move_session();
        // origin = [10, 20, 300, 200]，光标从 (100,100) 走到 (50,50)：位移 -50,-50
        let r = d.apply([50.0, 50.0], 0.016, 2.0, [24.0, 24.0]);
        assert_eq!(
            r,
            [-40.0, -30.0, 300.0, 200.0],
            "移动时尺寸必须原样返回（不受 scale 影响）"
        );
    }

    #[test]
    fn velocity_ema_converges_to_constant_speed() {
        let mut d = move_session();
        for i in 1..=200 {
            let t = i as f64 * 0.008;
            d.apply([100.0 + i as f32 * 8.0, 100.0], t, 1.0, [24.0, 24.0]); // 1000 px/s
        }
        let v = d.velocity();
        assert!((v[0] - 1000.0).abs() < 1.0, "vel={v:?}");
        assert!(v[1].abs() < 1e-3);
    }

    #[test]
    fn zero_dt_is_clamped_not_panics() {
        let mut d = move_session();
        let _ = d.apply([105.0, 105.0], 0.0, 1.0, [24.0, 24.0]); // dt = 0 -> MIN_DT
        assert!(
            d.velocity()[0].is_finite(),
            "dt=0 时速度必须是有限值，不能除零"
        );
    }

    #[test]
    fn resize_right_edge_grows_width_only() {
        let mut d = resize_session(E::RIGHT);
        let r = d.apply([50.0, 0.0], 0.016, 1.0, [24.0, 24.0]);
        assert_eq!(r, [100.0, 100.0, 350.0, 200.0]);
    }

    #[test]
    fn resize_left_edge_anchors_right_side() {
        let mut d = resize_session(E::LEFT);
        let r = d.apply([-50.0, 0.0], 0.016, 1.0, [24.0, 24.0]);
        assert_eq!(
            r,
            [50.0, 100.0, 350.0, 200.0],
            "左移 50 → x-50、w+50，右边不动"
        );
        assert!((r[0] + r[2] - 400.0).abs() < 1e-4);
    }

    #[test]
    fn resize_bottom_edge_grows_height_only() {
        let mut d = resize_session(E::BOTTOM);
        let r = d.apply([0.0, 30.0], 0.016, 1.0, [24.0, 24.0]);
        assert_eq!(r, [100.0, 100.0, 300.0, 230.0]);
    }

    #[test]
    fn resize_top_edge_anchors_bottom_side() {
        let mut d = resize_session(E::TOP);
        let r = d.apply([0.0, -30.0], 0.016, 1.0, [24.0, 24.0]);
        assert_eq!(r, [100.0, 70.0, 300.0, 230.0]);
        assert!((r[1] + r[3] - 300.0).abs() < 1e-4);
    }

    #[test]
    fn resize_corner_changes_both_dims() {
        let mut d = resize_session(E::BOTTOM_RIGHT);
        let r = d.apply([40.0, 25.0], 0.016, 1.0, [24.0, 24.0]);
        assert_eq!(r, [100.0, 100.0, 340.0, 225.0]);
        let mut d = resize_session(E::TOP_LEFT);
        let r = d.apply([-40.0, -25.0], 0.016, 1.0, [24.0, 24.0]);
        assert_eq!(r, [60.0, 75.0, 340.0, 225.0]);
    }

    #[test]
    fn resize_clamps_at_min_size_and_keeps_anchor_fixed() {
        // 抓左边往**右**拖过头：宽度夹到 min，右边（锚定边）必须纹丝不动
        let mut d = resize_session(E::LEFT);
        let r = d.apply([1000.0, 0.0], 0.016, 1.0, [120.0, 120.0]);
        assert_eq!(r[2], 120.0, "夹到最小宽");
        assert!((r[0] - 280.0).abs() < 1e-4, "x={}", r[0]);
        assert!((r[0] + r[2] - 400.0).abs() < 1e-4, "右边必须纹丝不动");
        // 抓右边往**左**拖过头：左边（锚定边）不动
        let mut d = resize_session(E::RIGHT);
        let r = d.apply([-1000.0, 0.0], 0.016, 1.0, [120.0, 120.0]);
        assert_eq!(r[2], 120.0);
        assert_eq!(r[0], 100.0, "左边不动");
        // 抓上边往下拖过头：下边不动
        let mut d = resize_session(E::TOP);
        let r = d.apply([0.0, 1000.0], 0.016, 1.0, [120.0, 120.0]);
        assert_eq!(r[3], 120.0);
        assert!(
            (r[1] + r[3] - 300.0).abs() < 1e-4,
            "下边必须纹丝不动, y={}",
            r[1]
        );
    }

    #[test]
    fn resize_min_size_floor_is_enforced() {
        // 上层给的 min 比 MIN_SIZE 还小时，取 MIN_SIZE（库的硬下限）
        let mut d = resize_session(E::BOTTOM_RIGHT);
        let r = d.apply([-1e6, -1e6], 0.016, 1.0, [1.0, 1.0]);
        assert!(r[2] >= crate::core::block::MIN_SIZE);
        assert!(r[3] >= crate::core::block::MIN_SIZE);
    }

    #[test]
    fn resize_converts_cursor_delta_by_dpi_scale() {
        // 200% DPI：光标走 100 物理 px = 50 逻辑 px
        let mut d = resize_session(E::RIGHT);
        let r = d.apply([100.0, 0.0], 0.016, 2.0, [24.0, 24.0]);
        assert!((r[2] - 350.0).abs() < 1e-3, "w={}", r[2]);
    }

    #[test]
    fn resize_survives_bad_scale() {
        for bad in [0.0f32, -1.0, f32::NAN, f32::INFINITY] {
            let mut d = resize_session(E::RIGHT);
            let r = d.apply([50.0, 0.0], 0.016, bad, [24.0, 24.0]);
            assert!(r.iter().all(|v| v.is_finite()), "scale={bad} -> {r:?}");
        }
    }

    #[test]
    fn session_introspection() {
        let m = move_session();
        assert_eq!(m.id(), 7);
        assert!(!m.is_resize());
        assert_eq!(m.edges(), None);
        let mut r = resize_session(E::TOP_RIGHT);
        assert!(r.is_resize());
        assert_eq!(r.edges(), Some(E::TOP_RIGHT));
        assert_eq!(r.id(), 7);
        let _ = r.apply([10.0, -10.0], 0.016, 1.0, [24.0, 24.0]);
        assert!(r.velocity()[1].is_finite());
    }
}
