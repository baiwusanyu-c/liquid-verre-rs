//! 样式缓存：把「逻辑样式 + DPI 比例」展开成渲染用的 [`LiquidParams`]，并跨帧复用。
//!
//! 为什么需要缓存：`LiquidParams::sanitize()` 有约 40 次 clamp、`scaled()` 还要再走一遍，
//! 20 块 × 每帧 = 白烧 `prep` 预算（`prep` 是丢帧判据 own_us 的一部分，不变量 8）。
//! 缓存以 `(块 id, 样式版本号, DPI 位模式)` 为键，只有样式或所在屏 DPI 真变了才重算，
//! 命中时只是一次 ~180 字节的 `Copy`。

use crate::api::config::GlassStyle;
use crate::core::params::LiquidParams;

#[derive(Debug, Clone, Copy)]
struct Entry {
    id: u32,
    epoch: u64,
    scale_bits: u32,
    params: LiquidParams,
}

/// 逐块样式展开缓存。热路径零堆分配（`Vec` 只在块数增长时扩容）。
#[derive(Debug, Default)]
pub(crate) struct StyleCache {
    entries: Vec<Entry>,
}

impl StyleCache {
    /// 取出某块在当前 DPI 下的有效参数。
    ///
    /// `scale` 为该屏 DPI 比例；`epoch` 为块的样式版本号（每次 `set_style` 递增）。
    pub(crate) fn resolve(
        &mut self,
        id: u32,
        epoch: u64,
        scale: f32,
        style: &GlassStyle,
    ) -> LiquidParams {
        let scale_bits = scale.to_bits();
        if let Some(hit) = self
            .entries
            .iter()
            .find(|e| e.id == id && e.epoch == epoch && e.scale_bits == scale_bits)
        {
            return hit.params;
        }
        let mut params = style.scaled(scale).to_params();
        // scaled() 内部已 sanitize；透明模式是 inert() 底座 + 覆盖三个字段，
        // 这里再夹一次，防止上层把 corner_radius 推到 400 后再乘 DPI 越界。
        params.sanitize();
        match self.entries.iter_mut().find(|e| e.id == id) {
            Some(slot) => {
                *slot = Entry {
                    id,
                    epoch,
                    scale_bits,
                    params,
                }
            }
            None => self.entries.push(Entry {
                id,
                epoch,
                scale_bits,
                params,
            }),
        }
        params
    }

    /// 丢弃某块的缓存（块被销毁时调用，避免 id 复用后拿到陈旧参数）。
    pub(crate) fn forget(&mut self, id: u32) {
        self.entries.retain(|e| e.id != id);
    }

    /// 清空（显示器重建 / DPI 变化时）。
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    /// 当前缓存条目数。只在测试里断言「同键不堆积」，因此不进 release 构建。
    #[cfg(test)]
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::StyleCache;
    use crate::api::config::{GlassStyle, TransparentParams};

    #[test]
    fn resolve_scales_pixel_dims_by_dpi() {
        let mut c = StyleCache::default();
        let s = GlassStyle::transparent(16.0, 0.5, [1.0; 4]);
        let p = c.resolve(1, 0, 2.0, &s);
        assert!(
            (p.corner_radius - 32.0).abs() < 1e-3,
            "r={}",
            p.corner_radius
        );
        assert!((p.base_alpha - 0.5).abs() < 1e-6);
    }

    #[test]
    fn cache_hit_returns_identical_params() {
        let mut c = StyleCache::default();
        let s = GlassStyle::default();
        let a = c.resolve(7, 3, 1.5, &s);
        let b = c.resolve(7, 3, 1.5, &s);
        assert_eq!(a, b);
        assert_eq!(c.len(), 1, "同键必须命中同一条目，不能堆积");
    }

    #[test]
    fn epoch_or_scale_change_recomputes() {
        let mut c = StyleCache::default();
        let s = GlassStyle::transparent(16.0, 0.5, [1.0; 4]);
        let a = c.resolve(1, 0, 1.0, &s);
        let s2 = GlassStyle::transparent(40.0, 0.5, [1.0; 4]);
        let b = c.resolve(1, 1, 1.0, &s2);
        assert!((b.corner_radius - 40.0).abs() < 1e-3);
        assert!((a.corner_radius - 16.0).abs() < 1e-3);
        // DPI 变了但样式没变，也必须重算
        let d = c.resolve(1, 1, 2.0, &s2);
        assert!((d.corner_radius - 80.0).abs() < 1e-3);
        assert_eq!(c.len(), 1, "同一个 id 只保留一条");
    }

    #[test]
    fn distinct_ids_do_not_collide() {
        let mut c = StyleCache::default();
        let s1 = GlassStyle::transparent(10.0, 0.2, [1.0, 0.0, 0.0, 1.0]);
        let s2 = GlassStyle::transparent(20.0, 0.8, [0.0, 0.0, 1.0, 1.0]);
        let p1 = c.resolve(1, 0, 1.0, &s1);
        let p2 = c.resolve(2, 0, 1.0, &s2);
        assert!((p1.corner_radius - 10.0).abs() < 1e-3);
        assert!((p2.corner_radius - 20.0).abs() < 1e-3);
        assert_eq!(c.len(), 2);
        c.forget(1);
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn corner_radius_is_clamped_after_scaling() {
        let mut c = StyleCache::default();
        let s = GlassStyle::Transparent(TransparentParams {
            corner_radius: 400.0,
            opacity: 1.0,
            color: [1.0; 4],
        });
        let p = c.resolve(1, 0, 4.0, &s);
        assert!(
            p.corner_radius <= 400.0,
            "sanitize 必须夹住 DPI 放大后的越界值"
        );
    }
}
