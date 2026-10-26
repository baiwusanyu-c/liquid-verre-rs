//! 内容层（P0-1）的**纯计算**：参数校验决策 + dirty 半开区间裁剪。
//!
//! 为什么单独成 core 模块：按项目规则（core/ 不依赖 windows/、0 unsafe、全部可单测），
//! `api/handle.rs` 的校验矩阵与 `gfx/content_layer.rs` 的子区上传边界是同一条验证链路
//! 的两端，判定逻辑必须只有一份（单一来源），且 AC2 要求它可在无 GPU 环境下穷举测试。
//!
//! 坐标系约定（与 FR1 对齐）：
//! - dirty 为**块内物理像素半开区间** [x0, y0, x1, y1)，`x1 > x0` 才非退化；
//! - 纹理与块矩形 1:1 且锚定左上角 ⇒ dirty 直接就是纹理空间坐标，无需缩放映射；
//! - 脏区对渲染失效的贡献 = 块矩形本身（内容永远画在块矩形内，C4 统一收口，
//!   不新增第二套 dirty 账本）。

/// 内容纹理单边上限（像素）。超过 ⇒ 丢弃该条 + warn-once（FR6）。
pub(crate) const MAX_CONTENT_DIM: u32 = 8192;

/// `update_content` 在 api 侧的校验结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentPlan {
    /// w 或 h 为 0：语义等价 `clear_content`。
    Clear,
    /// 校验通过，buffer 可拷贝入队。
    Upload,
    /// 尺寸超上限（> [`MAX_CONTENT_DIM`]），丢弃。
    TooLarge,
    /// buffer 长度 != w*h*4，丢弃。
    LenMismatch,
}

/// 校验决策（纯函数，AC2 单测入口）。
///
/// 判定顺序有意为之：0 尺寸先判（等价 clear，宿主用它表达「撤回内容」），
/// 再判上限（防溢出乘法），最后判长度（需要 w*h 参与计算）。
#[must_use]
pub(crate) fn plan_update(w_phys: u32, h_phys: u32, buffer_len: usize) -> ContentPlan {
    if w_phys == 0 || h_phys == 0 {
        return ContentPlan::Clear;
    }
    if w_phys > MAX_CONTENT_DIM || h_phys > MAX_CONTENT_DIM {
        return ContentPlan::TooLarge;
    }
    let expected = (w_phys as usize)
        .checked_mul(h_phys as usize)
        .and_then(|n| n.checked_mul(4));
    if expected == Some(buffer_len) {
        ContentPlan::Upload
    } else {
        ContentPlan::LenMismatch
    }
}

/// 上传区域的裁剪结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UploadRegion {
    /// 全量上传（dirty=None，语义：整面刷新）。
    Full,
    /// 只上传裁剪后的子矩形（半开区间，纹理空间）。
    Rect { x0: u32, y0: u32, x1: u32, y1: u32 },
    /// 裁剪后为空 ⇒ 没有任何像素要写。
    Empty,
}

/// 把宿主给的 dirty 半开区间裁剪进 [0,w)×[0,h) 的纹理边界。
///
/// 不可信输入纪律（spec：一切来自上层的数值先 sanitize）：坐标先 clamp，
/// 裁剪后 `x1 <= x0 || y1 <= y0` ⇒ [`UploadRegion::Empty`]（退化矩形丢弃，
/// 与 capture 侧 `sanitize_rect` 同一判据）。`None` 表示全量，不是「没有脏区」。
#[must_use]
pub(crate) fn clip_dirty(dirty: Option<[u32; 4]>, w: u32, h: u32) -> UploadRegion {
    let Some([x0, y0, x1, y1]) = dirty else {
        return UploadRegion::Full;
    };
    let cx0 = x0.min(w);
    let cy0 = y0.min(h);
    let cx1 = x1.min(w);
    let cy1 = y1.min(h);
    if cx1 <= cx0 || cy1 <= cy0 {
        return UploadRegion::Empty;
    }
    UploadRegion::Rect {
        x0: cx0,
        y0: cy0,
        x1: cx1,
        y1: cy1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn bytes(w: u32, h: u32) -> usize {
        (w as usize) * (h as usize) * 4
    }

    #[test]
    fn plan_zero_size_is_clear() {
        // w/h 为 0 ⇒ 等价 clear（FR6：0 尺寸不是错误，是「撤回」）
        assert_eq!(plan_update(0, 100, 0), ContentPlan::Clear);
        assert_eq!(plan_update(100, 0, 0), ContentPlan::Clear);
        assert_eq!(plan_update(0, 0, bytes(0, 0)), ContentPlan::Clear);
        // 0 尺寸优先于其它判定：即使长度/上限看着更「错」，语义仍是 clear
        assert_eq!(plan_update(0, 9000, 12345), ContentPlan::Clear);
    }

    #[test]
    fn plan_oversize_is_discarded() {
        assert_eq!(
            plan_update(8193, 10, bytes(8193, 10)),
            ContentPlan::TooLarge
        );
        assert_eq!(
            plan_update(10, 8193, bytes(10, 8193)),
            ContentPlan::TooLarge
        );
        // 恰好 8192 合法（上限是闭区间）
        assert_eq!(
            plan_update(8192, 8192, bytes(8192, 8192)),
            ContentPlan::Upload
        );
    }

    #[test]
    fn plan_length_mismatch_is_discarded() {
        assert_eq!(plan_update(100, 50, bytes(100, 50)), ContentPlan::Upload);
        // 多一字节 / 少一字节 / 少一整行都必须被识别，且不得 panic（乘法溢出防护）
        assert_eq!(
            plan_update(100, 50, bytes(100, 50) + 1),
            ContentPlan::LenMismatch
        );
        assert_eq!(
            plan_update(100, 50, bytes(100, 50) - 1),
            ContentPlan::LenMismatch
        );
        assert_eq!(plan_update(100, 50, 400), ContentPlan::LenMismatch);
        assert_eq!(plan_update(100, 50, 0), ContentPlan::LenMismatch);
    }

    #[test]
    fn plan_survives_extreme_without_overflow() {
        // 8192x8192x4 = 134_217_728，usize 安全；重点是 debug 构建不 panic（坑 7）
        assert_eq!(plan_update(u32::MAX, 1, 4), ContentPlan::TooLarge);
        assert_eq!(
            plan_update(8192, 8192, usize::MAX),
            ContentPlan::LenMismatch
        );
    }

    #[test]
    fn clip_none_is_full() {
        assert_eq!(clip_dirty(None, 100, 100), UploadRegion::Full);
    }

    #[test]
    fn clip_keeps_interior_and_clamps_overflow() {
        assert_eq!(
            clip_dirty(Some([10, 20, 30, 40]), 100, 100),
            UploadRegion::Rect {
                x0: 10,
                y0: 20,
                x1: 30,
                y1: 40
            }
        );
        // 越界端点被 clamp 到纹理边界（半开区间：x1 == w 合法）
        assert_eq!(
            clip_dirty(Some([0, 0, 500, 500]), 100, 80),
            UploadRegion::Rect {
                x0: 0,
                y0: 0,
                x1: 100,
                y1: 80
            }
        );
        assert_eq!(
            clip_dirty(Some([90, 70, 1000, 1000]), 100, 80),
            UploadRegion::Rect {
                x0: 90,
                y0: 70,
                x1: 100,
                y1: 80
            }
        );
    }

    #[test]
    fn clip_degenerate_and_outside_is_empty() {
        // 退化（x1 <= x0）与完全越界 ⇒ Empty，绝不返回 Rect{x1 < x0} 污染上传循环
        assert_eq!(
            clip_dirty(Some([50, 0, 50, 10]), 100, 100),
            UploadRegion::Empty
        );
        assert_eq!(
            clip_dirty(Some([50, 0, 40, 10]), 100, 100),
            UploadRegion::Empty
        );
        assert_eq!(
            clip_dirty(Some([200, 0, 300, 10]), 100, 100),
            UploadRegion::Empty
        );
        assert_eq!(
            clip_dirty(Some([0, 500, 10, 600]), 100, 100),
            UploadRegion::Empty
        );
        // 起点 clamp 到边界后再判退化
        assert_eq!(
            clip_dirty(Some([100, 0, 300, 10]), 100, 100),
            UploadRegion::Empty
        );
    }

    #[test]
    fn clip_on_zero_dims_texture_is_empty_or_full() {
        // w/h=0 的层不会进到这里（api 侧已转 clear），纯防御：不得 panic
        assert_eq!(clip_dirty(Some([0, 0, 1, 1]), 0, 0), UploadRegion::Empty);
        assert_eq!(clip_dirty(None, 0, 0), UploadRegion::Full);
    }
}
