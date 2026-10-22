//! 玻璃块（= 一个玻璃窗口）的数据模型。
//!
//! 全部字段都是标量 / `Copy` 类型，多屏投影每帧复制块时不会产生堆分配
//! （方案 7.3「热路径零堆分配」）。

use crate::api::config::{GlassStyle, ResizeOptions};

/// 块的最小边长（逻辑像素）。
pub const MIN_SIZE: f32 = 24.0;
/// 块的最大边长（逻辑像素）。
pub const MAX_SIZE: f32 = 8000.0;

/// 引擎内部持有的块记录（**逻辑单位**，未做 DPI 缩放）。
///
/// 渲染用的投影副本见 [`crate::core::draw::DrawBlock`]。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Block {
    /// 稳定 id，由 `Manager` 单调分配，上层通过 `GlassWindow` 句柄持有。
    pub id: u32,
    /// 左上角（虚拟屏物理 px，多屏绝对坐标）。
    pub x: f32,
    pub y: f32,
    /// 尺寸（**所在屏物理 px**，不含阴影边距）。
    pub width: f32,
    pub height: f32,
    /// 画布内叠放次序，越大越靠前。
    pub z_order: i32,
    /// 所在显示器下标（方案 12.10：默认钳制到单屏，不跨屏）。
    pub monitor: usize,
    /// 该屏 DPI / 主屏 DPI。块跨屏时按它换算，保持物理观感一致。
    pub scale: f32,
    /// 外观（模式 + 参数，逻辑单位）。
    pub style: GlassStyle,
    /// 是否继承引擎默认样式。
    ///
    /// 为 `true` 时本块的**有效样式**等于引擎当前默认值（见 `effective_style`）。
    /// `Engine::set_default_style` 只递增 `style_epoch` 让样式缓存作废，**不**改写
    /// `style` 字段本身（保持单一口径，避免两处各存一份「当前默认」）。
    /// 为 `false` 表示上层显式钉住了样式，不受默认值影响。
    pub inherits_style: bool,
    /// 外观版本号；变化时渲染侧才重算 DPI 缩放后的有效参数（避免每帧 sanitize）。
    pub style_epoch: u64,
    /// 缩放交互配置。
    pub resize: ResizeOptions,
    /// 是否可被鼠标命中。
    pub interactive: bool,
    /// 是否可拖拽移动位置（缩放把手不受它影响）。
    pub movable: bool,
    /// 是否可见。
    pub visible: bool,
    /// 是否被「z 序在我们之上的不透明窗口」**完整覆盖**（判据用绘制矩形，含阴影外扩）。
    ///
    /// **写点唯一**：`runtime::occlusion::update`（8 Hz 轮询，迟滞见 [`occlusion_step`]）。
    /// **读点三处**：
    /// 1. 主循环顶部的采集需求扫描（`active` / `wants_bd` 跳过被遮挡块）—— 于是
    ///    「本屏液态块全被遮挡」会落到现成的 `needs_backdrop_now == false` →
    ///    `idle_capturing` 停采路径（与全透明屏同一条，DDA 次/秒归零）；
    /// 2. `runtime::app::collect_monitor_blocks` 投影到 [`crate::core::draw::DrawBlock::occluded`]；
    /// 3. `mouse_relevant` 判定（被遮挡块的光标移动既不触发重画，也不把
    ///    `capture_wait_ms` 打到 0）。
    ///
    /// **不参与绘制集合**：`mon.blocks` / `cur_rects` 原样，因此单帧像素输出逐位不变
    /// （AC6 用 `GLASS_OCCLUSION=0/1` 同稳态帧对比来钉死这一点）。
    pub occluded: bool,
    /// 遮挡判定的连续命中计数（[`occlusion_step`] 的持久状态）。
    ///
    /// 与 `occluded` 同写同读（只有 `runtime::occlusion::update` 碰它）。单独存一个字段
    /// 是为了让「置位需连续 2 次、解除只需 1 次」这件事保持成**纯函数**：可单测、
    /// 无隐藏状态、也不会和 `occluded` 各自漂移（spec 的「单一来源」硬规矩）。
    /// `i8` 足够：它数到 `OCCLUSION_CONFIRM_POLLS` 就饱和，永远不会涨大。
    pub occl_streak: i8,
}

impl Block {
    pub(crate) fn new(id: u32, x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            id,
            x,
            y,
            width: width.max(MIN_SIZE),
            height: height.max(MIN_SIZE),
            z_order: id as i32,
            monitor: 0,
            scale: 1.0,
            style: GlassStyle::default(),
            inherits_style: true,
            style_epoch: 0,
            resize: ResizeOptions::default(),
            interactive: true,
            movable: true,
            visible: true,
            occluded: false,
            occl_streak: 0,
        }
    }

    /// 尺寸 / 位置夹紧。
    ///
    /// 位置下限取「半出屏」而不是 0：玻璃块允许一半在屏幕外（弹窗滑入动画），
    /// 但必须挡住 `±1e9` 这类非法值把 f32 精度吃光。
    pub(crate) fn clamp(&mut self) {
        self.width = self.width.clamp(MIN_SIZE, MAX_SIZE);
        self.height = self.height.clamp(MIN_SIZE, MAX_SIZE);
        self.x = clamp_finite_pos(self.x, -self.width * 0.5);
        self.y = clamp_finite_pos(self.y, -self.height * 0.5);
    }

    /// 本块的**有效样式**：继承默认样式的块必须读引擎当前默认值，
    /// 而不是创建那一刻的副本。
    ///
    /// 这是单一来源：渲染投影（`collect_monitor_blocks`）、快照发布
    /// （`publish_snapshot`）、采集门控（主循环的 `wants_bd`）三处都必须走它。
    /// 漏走的一处后果实测过：采集门控读陈旧 `b.style` 时，`set_default_style(transparent)`
    /// 之后整屏已是透明玻璃，`wants_bd` 仍为 true，DDA 采集继续全速跑 ——
    /// 正好抵消需求 F3.1「透明模式省采集」的收益（独立审查 P1-1a）。
    #[must_use]
    pub(crate) fn effective_style<'a>(&'a self, default: &'a GlassStyle) -> &'a GlassStyle {
        if self.inherits_style {
            default
        } else {
            &self.style
        }
    }

    /// 有效圆角半径（物理像素）：样式半径按 DPI 换算后，不超过短边的一半。
    ///
    /// `style` 由调用方传入而不是读 `self.style`：继承默认样式的块要先把
    /// 引擎默认值解析出来，`self.style` 在那之前可能是陈旧的。
    #[must_use]
    pub(crate) fn effective_radius(&self, style: &GlassStyle) -> f32 {
        (style.corner_radius() * self.scale).clamp(0.0, 0.5 * self.width.min(self.height))
    }
}

/// NaN 安全的位置夹紧（上限 20000，与重构前一致）。
fn clamp_finite_pos(v: f32, lo: f32) -> f32 {
    if v.is_finite() {
        v.clamp(lo, 20000.0)
    } else {
        lo.max(0.0)
    }
}

/// 遮挡确认所需的连续命中次数（8 Hz 轮询 ⇒ ≤250 ms）。
///
/// 为什么要迟滞：`EnumWindows` 拿到的窗口快照与我们的块几何不是同一时刻取的，
/// 遮挡窗口正在移动 / 正在创建销毁时单次判定会抖。置位慢一轮（+125 ms）只是让
/// 「省下来的采集」晚一点开始；解除快（1 次）保证遮挡一移开立刻恢复渲染 ——
/// 错判方向必须是「多渲染」而不是「少渲染」。
pub(crate) const OCCLUSION_CONFIRM_POLLS: i8 = 2;

/// 遮挡迟滞状态机：由上一轮的 `streak` 与本轮判定 `hit` 推出新的 `(occluded, streak)`。
///
/// - 置位需**连续** [`OCCLUSION_CONFIRM_POLLS`] 次命中；
/// - **解除只需 1 次未命中**（立即恢复），且 `streak` 归零；
/// - `streak` 数到阈值就饱和，不再自增：连续命中 1000 轮与 2 轮语义完全相同，
///   继续数只会让解除变慢。先 `clamp` 再 `saturating_add` 是为了杜绝 i8 溢出 ——
///   debug 构建下裸 `streak + 1` 在第 128 轮就 panic（spec 明令禁止裸的整数加减），
///   而 `streak` 是跨轮持久字段，脏值 / 阈值日后调大都可能把它推到边界。
#[must_use]
pub(crate) fn occlusion_step(streak: i8, hit: bool) -> (bool, i8) {
    if !hit {
        // 一次未命中就解除：宁可多渲染一帧，也不许让玻璃在遮挡移开后还是黑的。
        return (false, 0);
    }
    let next = streak
        .clamp(0, OCCLUSION_CONFIRM_POLLS)
        .saturating_add(1)
        .min(OCCLUSION_CONFIRM_POLLS);
    (next >= OCCLUSION_CONFIRM_POLLS, next)
}

#[cfg(test)]
mod tests {
    use super::{occlusion_step, Block, MAX_SIZE, MIN_SIZE, OCCLUSION_CONFIRM_POLLS};
    use crate::api::config::GlassStyle;

    #[test]
    fn new_blocks_start_unoccluded() {
        let b = Block::new(1, 0.0, 0.0, 100.0, 100.0);
        assert!(!b.occluded && b.occl_streak == 0);
    }

    #[test]
    fn occlusion_latches_only_after_two_consecutive_hits() {
        // 首次命中不置位（迟滞：EnumWindows 快照与块几何不同时刻取的，会抖）
        let (occ, streak) = occlusion_step(0, true);
        assert!(!occ);
        assert_eq!(streak, 1);
        // 连续第 2 次命中才置位
        let (occ, streak) = occlusion_step(streak, true);
        assert!(occ);
        assert_eq!(streak, OCCLUSION_CONFIRM_POLLS);
    }

    #[test]
    fn occlusion_releases_on_a_single_miss_and_resets_streak() {
        // 解除只需 1 次：错判方向必须是「多渲染」而不是「少渲染」
        let (occ, streak) = occlusion_step(OCCLUSION_CONFIRM_POLLS, false);
        assert!(!occ);
        assert_eq!(streak, 0);
        // 解除后必须重新数满 OCCLUSION_CONFIRM_POLLS 次才再置位（不许留残余计数）
        let (occ, streak) = occlusion_step(streak, true);
        assert!(!occ);
        assert_eq!(streak, 1);
    }

    #[test]
    fn occlusion_streak_saturates_and_never_overflows_i8() {
        // 先走正常路径置位（连续 2 次命中）
        let (occ, s1) = occlusion_step(0, true);
        assert!(!occ);
        assert_eq!(s1, 1);
        let (occ, mut streak) = occlusion_step(s1, true);
        assert!(occ, "连续 2 次命中后必须已置位");
        // 已置位之后再连续命中 1000 轮：debug 构建下裸 `streak + 1`
        // 会在第 128 轮溢出 panic（i8::MAX = 127）
        for round in 0..1000 {
            let (occ, next) = occlusion_step(streak, true);
            assert!(occ, "第 {} 轮：已置位后继续命中必须保持遮挡", round);
            assert_eq!(next, OCCLUSION_CONFIRM_POLLS, "streak 必须饱和而不是继续涨");
            streak = next;
        }
        // 饱和之后一次未命中仍然立刻解除
        assert_eq!(occlusion_step(streak, false), (false, 0));
    }

    #[test]
    fn occlusion_step_tolerates_dirty_streak() {
        // streak 是跨轮持久字段：脏值 / 阈值日后调大都不许让状态机跑飞或 panic
        let (occ, streak) = occlusion_step(i8::MAX, true);
        assert!(occ);
        assert_eq!(streak, OCCLUSION_CONFIRM_POLLS);
        let (occ, streak) = occlusion_step(-7, true);
        assert!(!occ, "负值按 0 起算，等价于第一次命中");
        assert_eq!(streak, 1);
        assert_eq!(occlusion_step(i8::MIN, false), (false, 0));
    }

    #[test]
    fn clamp_enforces_size_bounds() {
        let mut b = Block::new(1, 0.0, 0.0, 1e9, 1e9);
        b.clamp();
        assert_eq!(b.width, MAX_SIZE);
        assert_eq!(b.height, MAX_SIZE);
        let mut c = Block::new(2, 0.0, 0.0, 1.0, 1.0);
        c.clamp();
        assert_eq!(c.width, MIN_SIZE);
        assert_eq!(c.height, MIN_SIZE);
    }

    #[test]
    fn clamp_allows_half_offscreen_but_bounds_extreme() {
        let mut b = Block::new(3, -1e9, -1e9, 400.0, 300.0);
        b.clamp();
        assert!(b.x >= -b.width * 0.5);
        assert!(b.y >= -b.height * 0.5);
    }

    #[test]
    fn clamp_survives_nan_position() {
        let mut b = Block::new(4, f32::NAN, f32::NAN, 400.0, 300.0);
        b.clamp();
        assert!(b.x.is_finite() && b.y.is_finite());
    }

    #[test]
    fn radius_is_capped_by_short_side() {
        let b = Block::new(5, 0.0, 0.0, 100.0, 40.0);
        let s = GlassStyle::transparent(500.0, 0.5, [1.0; 4]);
        assert!(
            (b.effective_radius(&s) - 20.0).abs() < 1e-3,
            "r={}",
            b.effective_radius(&s)
        );
    }

    #[test]
    fn radius_scales_with_dpi() {
        let mut b = Block::new(6, 0.0, 0.0, 400.0, 400.0);
        let s = GlassStyle::transparent(16.0, 0.5, [1.0; 4]);
        b.scale = 2.0;
        assert!((b.effective_radius(&s) - 32.0).abs() < 1e-3);
    }

    #[test]
    fn effective_style_follows_the_engine_default_while_inheriting() {
        let mut b = Block::new(1, 0.0, 0.0, 400.0, 300.0);
        let d1 = GlassStyle::transparent(16.0, 0.5, [1.0; 4]);
        assert_eq!(b.effective_style(&d1).corner_radius(), 16.0);
        // 引擎默认变了，继承块的有效样式必须立刻跟着变（采集门控 / 渲染 / 快照
        // 三处都走这一个出口，漏一处就会出现「透明了但采集不降频」这类半态）
        let d2 = GlassStyle::transparent(40.0, 0.5, [1.0; 4]);
        assert_eq!(b.effective_style(&d2).corner_radius(), 40.0);
        // 钉住之后不再跟随
        b.style = GlassStyle::transparent(8.0, 0.5, [1.0; 4]);
        b.inherits_style = false;
        assert_eq!(b.effective_style(&d2).corner_radius(), 8.0);
    }

    #[test]
    fn new_blocks_are_movable_and_interactive_by_default() {
        let b = Block::new(1, 0.0, 0.0, 100.0, 100.0);
        assert!(b.movable && b.interactive && b.visible);
    }

    #[test]
    fn transparent_style_has_zero_shadow_margin() {
        // 外扩边距走 GlassStyle::shadow_margin()：透明模式必须为 0，
        // 否则窗口区域和脏区会白扩一圈。
        assert!(GlassStyle::default().shadow_margin() > 0.0);
        assert_eq!(
            GlassStyle::transparent(16.0, 0.5, [1.0; 4]).shadow_margin(),
            0.0
        );
    }
}
