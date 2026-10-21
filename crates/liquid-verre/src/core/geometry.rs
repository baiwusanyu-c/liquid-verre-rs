//! 矩形 / 影响盒运算的**单一来源**。
//!
//! 原来这些自由函数写在 `gfx/compositor.rs` 里，`runtime` 又各自手写了一遍影响盒
//! 外扩，属于 spec 明令禁止的「两处手写同一常量」。统一收到这里，
//! `gfx` / `runtime` / `platform` 都只调用不重写。
//!
//! 矩形约定：`[x0, y0, x1, y1]`，**半开区间**（`x1 > x0` 才非退化），单位为画布物理像素。

/// 影响盒半径（画布像素）：光标依赖项的**有限支撑窗**。
///
/// 必须与 `glass_ps.hlsl` 里 gate 的支撑半径 `3*mouse_radius` 严格对齐：
/// 盒外块的鼠标依赖精确为 0，盒内块整块重画，两者都不会出现拼接缝。
/// 单一来源，避免两处字面量漂移（不变量 5）。
#[must_use]
pub fn cursor_influence_px(mouse_radius: f32) -> i32 {
    (mouse_radius * 3.0 + 32.0) as i32
}

/// 缩放反馈（白色 border）的支撑半径（画布像素）。
///
/// border 只在「光标落在 handle 命中带内」时出现，命中带宽 = `grip_px * scale`。
/// 这里给出一个**严格更大**的有限支撑窗，让脏区影响盒一定能盖住它：
/// 盒外的块缩放反馈精确为 0（与不变量 5 同一套论证）。
#[must_use]
pub fn resize_influence_px(grip_px: f32) -> i32 {
    (grip_px.max(0.0) * 2.0 + 16.0) as i32
}

/// 帧脏区影响盒 = 光标项与缩放项的并集（取大者）。
#[must_use]
pub fn frame_influence_px(mouse_radius: f32, grip_px: f32) -> i32 {
    cursor_influence_px(mouse_radius).max(resize_influence_px(grip_px))
}

/// `outer` 是否完整包含 `inner`（闭区间比较，边界相等算包含）。
#[must_use]
pub fn rect_contains(outer: &[i32; 4], inner: &[i32; 4]) -> bool {
    outer[0] <= inner[0] && outer[1] <= inner[1] && outer[2] >= inner[2] && outer[3] >= inner[3]
}

/// 两个矩形的并集外接框（不做退化检查，调用方负责 clamp）。
#[must_use]
pub fn rect_union(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

/// 一组矩形的外接框；空集合返回 `None`。
#[must_use]
pub fn rects_bbox(rs: &[[i32; 4]]) -> Option<[i32; 4]> {
    rs.iter().copied().reduce(rect_union)
}

/// 相交判定（**开区间**：仅边界相接不算相交）。
#[must_use]
pub fn rects_hit(a: &[i32; 4], b: &[i32; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

/// 向四周外扩 `d` 像素。
#[must_use]
pub fn rect_inflate(r: [i32; 4], d: i32) -> [i32; 4] {
    [r[0] - d, r[1] - d, r[2] + d, r[3] + d]
}

/// clamp 到 `[0, w] x [0, h]`；结果可能退化（`x1 <= x0`），调用方需自行判空。
#[must_use]
pub fn rect_clamp(r: [i32; 4], w: i32, h: i32) -> [i32; 4] {
    [
        r[0].clamp(0, w),
        r[1].clamp(0, h),
        r[2].clamp(0, w),
        r[3].clamp(0, h),
    ]
}

/// 矩形面积（退化矩形返回 0，不会出现负数）。
#[must_use]
pub fn rect_area(r: &[i32; 4]) -> i64 {
    let w = (r[2] - r[0]).max(0) as i64;
    let h = (r[3] - r[1]).max(0) as i64;
    w * h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_is_inclusive() {
        assert!(rect_contains(&[0, 0, 10, 10], &[0, 0, 10, 10]));
        assert!(rect_contains(&[0, 0, 10, 10], &[2, 3, 4, 5]));
        assert!(!rect_contains(&[0, 0, 10, 10], &[0, 0, 11, 10]));
    }

    #[test]
    fn union_is_commutative_and_associative() {
        let a = [0, 0, 5, 5];
        let b = [3, 7, 9, 9];
        assert_eq!(rect_union(a, b), rect_union(b, a));
        assert_eq!(rect_union(a, b), [0, 0, 9, 9]);
    }

    #[test]
    fn bbox_of_empty_is_none() {
        assert_eq!(rects_bbox(&[]), None);
        assert_eq!(rects_bbox(&[[1, 2, 3, 4]]), Some([1, 2, 3, 4]));
    }

    #[test]
    fn hit_is_open_interval() {
        assert!(!rects_hit(&[0, 0, 5, 5], &[5, 0, 9, 5]));
        assert!(rects_hit(&[0, 0, 5, 5], &[4, 0, 9, 5]));
    }

    #[test]
    fn inflate_and_clamp() {
        assert_eq!(rect_inflate([5, 5, 10, 10], 2), [3, 3, 12, 12]);
        assert_eq!(rect_clamp([-5, 3, 999, 20], 100, 50), [0, 3, 100, 20]);
    }

    #[test]
    fn area_never_negative() {
        assert_eq!(rect_area(&[10, 10, 0, 0]), 0);
        assert_eq!(rect_area(&[0, 0, 4, 5]), 20);
    }

    #[test]
    fn influence_boxes_strictly_cover_shader_support() {
        // gate 支撑 = 3*mouse_radius；影响盒必须严格更大（还有 32px 余量）
        for r in [20.0f32, 220.0, 1200.0] {
            assert!(cursor_influence_px(r) as f32 > r * 3.0);
        }
        // 缩放反馈支撑 = grip（命中带）；影响盒必须严格更大
        for g in [0.0f32, 10.0, 64.0] {
            assert!(resize_influence_px(g) as f32 > g);
        }
        assert_eq!(
            frame_influence_px(220.0, 10.0),
            cursor_influence_px(220.0),
            "mouse_radius 默认档下光标项应当主导"
        );
        assert_eq!(
            frame_influence_px(20.0, 400.0),
            resize_influence_px(400.0),
            "极大 grip 下缩放项应当主导"
        );
    }
}
