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
    /// 是否被「z 序在我们之上的不透明窗口」完整覆盖（投影自
    /// [`crate::core::block::Block::occluded`]，8 Hz 更新，滞后 ≤1 圈）。
    ///
    /// **唯一消费者是主循环的 `mouse_relevant` 判定**：被遮挡块的光标移动不该触发重画，
    /// 更不该把 `capture_wait_ms` 打到 0 —— 后者会让采集从「在驱动里阻塞」退化成
    /// 满频轮询，正是 CPU 11.87% / 16.25% 那一档的成因之一（`performance_report.md`
    /// 2026-10-21 健康窗口实测）。
    ///
    /// **不影响绘制**：块照常进 `mon.blocks` 与 `cur_rects`，`draw_rect` / `margin` /
    /// `needs_backdrop` 都不读它，因此单帧像素输出逐位不变（AC6）。
    pub occluded: bool,
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

/// border 的发射计划：(块下标, 该块玻璃在 order 里的序号)，按玻璃序号升序。
///
/// 这是 0.5.2 #1「border 属于块本身，受块 z 序遮挡」这条语义的**单一判据**：
/// border 必须紧跟自己那块玻璃，且早于任何比它高的块的玻璃。历史上 border 是
/// draw_blocks 之后的独立叠加 pass（对所有块都是「后画 = 压在上面」），两块
/// 重叠时下层框的白边会画到上层框身上（用户复验：修复失败）。
/// ⚠ 0.5.3 #11 更正措辞：当年「抬层绕法（宿主 raise_box）修不了」的原因是
/// **把手命中判据缺遮挡项**（hover 判给了下层块，见 `core::manager::hit_resize` 与
/// BASELINE 不变量 13），不是「抬层能力不够」—— 别把这句读成「抬层仍然可以绕」。
///
/// 计划本身不含 GPU 状态，因此可以脱离渲染线程单测；gfx::glass_renderer 在
/// 逐块循环里按序号消费它，两者共用这一份定义（不再有两处手写同一规则）。
///
/// has_feedback(block_index) 由调用方提供（投影层决定哪块被 hover、把手是否
/// 命中），这里只回答「哪些块要发 border、发在第几次玻璃之后」。
pub(crate) fn border_emit_plan(
    order: &[usize],
    has_feedback: impl Fn(usize) -> bool,
    plan: &mut Vec<(usize, usize)>,
) {
    debug_assert!(
        plan.is_empty(),
        "调用方必须先 clear()：本函数只追加，不重置（避免两处各有一套清空语义）"
    );
    for (seq, &index) in order.iter().enumerate() {
        if has_feedback(index) {
            plan.push((index, seq));
        }
    }
}

/// 测试专用包装：把 [`border_emit_plan`] 收进一个新 Vec，方便逐条断言。
#[cfg(test)]
pub(crate) fn border_emit_plan_vec(
    order: &[usize],
    has_feedback: impl Fn(usize) -> bool,
) -> Vec<(usize, usize)> {
    let mut plan = Vec::new();
    border_emit_plan(order, has_feedback, &mut plan);
    plan
}

#[cfg(test)]
mod tests {
    use super::{border_emit_plan_vec, DrawBlock, ResizeFeedback};
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
            occluded: false,
            feedback: ResizeFeedback::NONE,
        }
    }

    #[test]
    fn occluded_does_not_change_draw_geometry() {
        // B1 只摘「触发源」，**不改绘制集合**：occluded 翻转不许影响
        // draw_rect / margin / needs_backdrop，否则「单帧像素输出逐位不变」（AC6）
        // 就不成立了 —— 这条单测是那个承诺的回归网。
        let b = mk(GlassMode::Liquid, 300.0, 200.0);
        let mut a = b;
        a.occluded = true;
        assert!(!b.occluded);
        assert_eq!(a.draw_rect(), b.draw_rect());
        assert_eq!(a.margin(), b.margin());
        assert_eq!(a.needs_backdrop(), b.needs_backdrop());
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

    // ---- 0.5.2 #1：border 层序语义（无 GPU 纯函数判据）----

    /// 断言 1：只有带 feedback 的块进计划。
    #[test]
    fn plan_only_includes_blocks_with_feedback() {
        // order = 玻璃绘制序（升序 = 从低到高）；块 1、3 有 feedback。
        let order = [0usize, 1, 2, 3, 4];
        let plan = border_emit_plan_vec(&order, |idx| idx == 1 || idx == 3);
        assert_eq!(plan, vec![(1, 1), (3, 3)], "计划必须只含带 feedback 的块");
    }

    /// 断言 2：border 紧跟自己块的玻璃序号，且早于任何更高块的玻璃序号。
    ///
    /// 这条就是本次主诉的数学表述：旧实现把 border 放在全部玻璃之后，
    /// 于是下层块的 border 序号永远大于上层块的玻璃序号 = 压在上面。
    #[test]
    fn border_follows_its_own_glass_and_precedes_higher_blocks() {
        let order = [7usize, 2, 9]; // 玻璃序：块7 最低、块9 最高
        let plan = border_emit_plan_vec(&order, |idx| idx == 7);
        assert_eq!(plan, vec![(7, 0)]);
        let own_seq = plan[0].1;
        // 自己块的玻璃之后、任何更高块的玻璃之前
        for (seq, &idx) in order.iter().enumerate() {
            if idx != 7 {
                assert!(
                    own_seq < seq,
                    "block {idx} 的玻璃(seq={seq}) 必须晚于块 7 的 border(seq={own_seq})"
                );
            }
        }
    }

    /// 断言 3：两块同时有 feedback 时，顺序跟随 order（painter's order）。
    #[test]
    fn plan_order_follows_block_order_with_multiple_feedbacks() {
        let order = [3usize, 1, 2];
        let plan = border_emit_plan_vec(&order, |idx| idx == 1 || idx == 3);
        // order = [3,1,2] ⇒ 块 3 是第 0 次玻璃、块 1 是第 1 次、块 2 是第 2 次。
        assert_eq!(
            plan,
            vec![(3, 0), (1, 1)],
            "border 序必须跟随玻璃序（order 的位置），而不是块号或插入序"
        );
        assert!(
            plan.as_slice().windows(2).all(|w| w[0].1 < w[1].1),
            "计划必须按玻璃序号严格升序，渲染侧才能用游标消费"
        );
    }

    /// 断言 4（判别力护栏）：无 feedback 时计划为空 = 渲染侧零状态切换。
    #[test]
    fn plan_is_empty_when_no_block_has_feedback() {
        let order = [0usize, 1, 2];
        assert!(
            border_emit_plan_vec(&order, |_| false).is_empty(),
            "没有反馈块时计划必须为空（这是「零切换」承诺的前提）"
        );
        // 越界下标不参与：order 只能引用真实存在的块，这里用「回调说 false」表达
        assert!(
            border_emit_plan_vec(&[], |_| true).is_empty(),
            "空 order 不产生计划"
        );
    }
}
