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

#[cfg(test)]
mod tests {
    use super::{Block, MAX_SIZE, MIN_SIZE};
    use crate::api::config::GlassStyle;

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
