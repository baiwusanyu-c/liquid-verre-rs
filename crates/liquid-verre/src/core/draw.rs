//! 渲染用的**投影块**：`core::block::Block`（逻辑单位）按所在屏 DPI 展开后的形态。
//!
//! 为什么要单独一个类型：
//! - `Block` 存的是**逻辑**样式（未缩放），跨屏时要按 DPI 换算；
//! - 渲染层需要的是**已经缩放 + sanitize 过**的参数，且不想每帧重算
//!   （`sanitize` 有 ~40 次 clamp，20 块 × 每帧 = 白烧 prep 预算）；
//! - 于是投影时按 `style_epoch` 命中缓存，只有样式真的变过才重算。
//!
//! 该类型是 `Copy`（全是标量 / 定长数组），逐帧复制不产生堆分配。

use crate::api::config::GlassMode;
use crate::core::params::LiquidParams;

/// 缩放悬停反馈（加粗白色 border）的逐块数据。
///
/// **只有被 hover 的那一块**会拿到非零值：其余块全 0，着色器里 `fb` 精确为 0，
/// 因此「拖一块、其余块逐位不变」的隔离性（不变量 5）不受影响。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct ResizeFeedback {
    /// 直边反馈开关（左 右 上 下），0/1。
    pub edges: [f32; 4],
    /// 对角反馈开关（左上 右上 左下 右下），0/1。
    pub corners: [f32; 4],
    /// border 粗细（物理像素）。
    pub width: f32,
    /// border 不透明度 0–1。
    pub alpha: f32,
    /// border 颜色。
    pub color: [f32; 3],
}

impl ResizeFeedback {
    /// 全零（不绘制任何反馈）。
    pub(crate) const NONE: ResizeFeedback = ResizeFeedback {
        edges: [0.0; 4],
        corners: [0.0; 4],
        width: 0.0,
        alpha: 0.0,
        color: [0.0; 3],
    };

    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        *self == Self::NONE || self.alpha <= 0.0
    }
}

/// 投影到某块显示器画布后的块（**画布局部坐标**）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DrawBlock {
    pub id: u32,
    /// 画布局部左上角（物理像素，不含阴影外扩）。
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// 有效圆角半径（物理像素，已按短边夹过）。
    pub radius: f32,
    pub z_order: i32,
    /// 所在屏 DPI 比例（着色器里做 AA 宽度等换算用）。
    pub scale: f32,
    /// 已按 DPI 缩放 + sanitize 的参数（`clamp_for_block` 仍在渲染时逐块做）。
    pub params: LiquidParams,
    /// 渲染模式。
    pub mode: GlassMode,
    /// 是否可被鼠标命中。
    ///
    /// 穿透判定（主循环的 `inside`）必须排除不可交互块，否则 `interactive=false`
    /// 的玻璃会把 `WS_EX_TRANSPARENT` 摘掉、跨进程吃掉点击（公共 API 承诺了
    /// 「关掉后完全点击穿透」）。注意**窗口区域仍要包含它** —— 它要画出来，
    /// 只是不接鼠标（独立审查 P1-4）。
    pub interactive: bool,
    /// 是否可拖拽移动（投影自 `Block::movable`，窗口过程据此拒绝开始移动会话）。
    pub movable: bool,
    /// 缩放反馈（仅 hover 块非零）。
    pub feedback: ResizeFeedback,
}

impl DrawBlock {
    /// 阴影外扩边距（物理像素）。
    #[must_use]
    pub(crate) fn margin(&self) -> f32 {
        self.params.shadow_margin.max(0.0)
    }

    /// 是否需要桌面采集 / 模糊金字塔。
    #[must_use]
    pub(crate) fn needs_backdrop(&self) -> bool {
        self.mode.needs_backdrop()
    }

    /// 绘制矩形（含阴影外扩），画布物理像素，`[x0,y0,x1,y1]` 半开区间。
    #[must_use]
    pub(crate) fn draw_rect(&self) -> [i32; 4] {
        let m = self.margin();
        let sx = (self.x - m).floor() as i32;
        let sy = (self.y - m).floor() as i32;
        let ex = sx + ((self.width + m * 2.0).ceil().max(1.0) as i32);
        let ey = sy + ((self.height + m * 2.0).ceil().max(1.0) as i32);
        [sx, sy, ex, ey]
    }
}

#[cfg(test)]
mod tests {
    use super::{DrawBlock, ResizeFeedback};
    use crate::api::config::{GlassMode, GlassStyle};
    use crate::core::params::LiquidParams;

    fn mk(mode: GlassMode, w: f32, h: f32) -> DrawBlock {
        let style = match mode {
            GlassMode::Liquid => GlassStyle::default(),
            GlassMode::Transparent => GlassStyle::transparent(16.0, 0.5, [1.0; 4]),
        };
        DrawBlock {
            id: 1,
            x: 10.5,
            y: 20.25,
            width: w,
            height: h,
            radius: 16.0,
            z_order: 0,
            scale: 1.0,
            params: style.to_params(),
            mode,
            interactive: true,
            movable: true,
            feedback: ResizeFeedback::NONE,
        }
    }

    #[test]
    fn transparent_has_zero_margin_and_skips_backdrop() {
        let t = mk(GlassMode::Transparent, 300.0, 200.0);
        assert_eq!(t.margin(), 0.0);
        assert!(!t.needs_backdrop());
    }

    #[test]
    fn liquid_keeps_shadow_margin_and_needs_backdrop() {
        let l = mk(GlassMode::Liquid, 300.0, 200.0);
        assert!(l.margin() > 0.0);
        assert!(l.needs_backdrop());
    }

    #[test]
    fn draw_rect_grows_by_margin_and_rounds_outward() {
        // x=10.5 y=20.25 w=300 h=200：floor/ceil 必须向外取整，
        // 否则块右下边缘会少画一行像素（历史上真出现过 1px 亮边）。
        let t = mk(GlassMode::Transparent, 300.0, 200.0);
        assert_eq!(t.draw_rect(), [10, 20, 310, 220], "透明块无阴影外扩");

        let l = mk(GlassMode::Liquid, 300.0, 200.0);
        let m = l.margin();
        assert!(m > 0.0, "液态块默认有阴影边距");
        let r = l.draw_rect();
        // 外扩后必须盖住「几何矩形 ⊕ margin」，允许 floor/ceil 组合带来的 <1px 余差
        // （与重构前 `rt_x = (x-margin).floor(); ex = rt_x + ceil(w+2m)` 的口径逐字一致，
        //   这半个像素落在 AA 衰减带里，是基线取证图已经接受的形状）。
        assert!(r[0] as f32 <= 10.5 - m, "left {}", r[0]);
        assert!(r[1] as f32 <= 20.25 - m, "top {}", r[1]);
        assert!(r[2] as f32 >= 10.5 + 300.0 + m - 1.0, "right {}", r[2]);
        assert!(r[3] as f32 >= 20.25 + 200.0 + m - 1.0, "bottom {}", r[3]);
        // 且不会白扩超过 1 像素（取整只向外一格）
        assert!((r[2] - r[0]) as f32 <= 300.0 + 2.0 * m + 1.0);
        assert!((r[3] - r[1]) as f32 <= 200.0 + 2.0 * m + 1.0);
    }

    #[test]
    fn feedback_none_is_empty() {
        assert!(ResizeFeedback::NONE.is_empty());
        let mut f = ResizeFeedback::NONE;
        f.alpha = 0.9;
        f.edges = [0.0, 1.0, 0.0, 0.0];
        assert!(!f.is_empty());
        f.alpha = 0.0;
        assert!(f.is_empty(), "alpha=0 等价于不绘制");
    }

    #[test]
    fn params_are_copy_so_projection_is_allocation_free() {
        let p = LiquidParams::default();
        let q = p;
        assert_eq!(p, q);
    }
}
