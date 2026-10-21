//! 缩放把手（8 个方向）的纯几何判定。
//!
//! 这是「边缘拖拽 / 对角拖拽」（需求 F2）的**唯一**判定点：
//! `input::resize` 的状态机、`core::manager` 的命中测试、`gfx::glass_renderer`
//! 的 border 反馈都只消费这里的结果，不再各自手写一遍阈值。

/// 参与缩放的边集合。
///
/// 单边 = 边缘拖拽（改一个维度）；相邻两边 = 对角拖拽（同时改两个维度）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) struct ResizeEdges {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

impl ResizeEdges {
    pub(crate) const NONE: ResizeEdges = ResizeEdges {
        left: false,
        right: false,
        top: false,
        bottom: false,
    };
    pub(crate) const LEFT: ResizeEdges = ResizeEdges {
        left: true,
        right: false,
        top: false,
        bottom: false,
    };
    pub(crate) const RIGHT: ResizeEdges = ResizeEdges {
        left: false,
        right: true,
        top: false,
        bottom: false,
    };
    pub(crate) const TOP: ResizeEdges = ResizeEdges {
        left: false,
        right: false,
        top: true,
        bottom: false,
    };
    pub(crate) const BOTTOM: ResizeEdges = ResizeEdges {
        left: false,
        right: false,
        top: false,
        bottom: true,
    };
    pub(crate) const TOP_LEFT: ResizeEdges = ResizeEdges {
        left: true,
        right: false,
        top: true,
        bottom: false,
    };
    pub(crate) const TOP_RIGHT: ResizeEdges = ResizeEdges {
        left: false,
        right: true,
        top: true,
        bottom: false,
    };
    pub(crate) const BOTTOM_LEFT: ResizeEdges = ResizeEdges {
        left: true,
        right: false,
        top: false,
        bottom: true,
    };
    pub(crate) const BOTTOM_RIGHT: ResizeEdges = ResizeEdges {
        left: false,
        right: true,
        top: false,
        bottom: true,
    };

    #[must_use]
    pub(crate) const fn is_empty(self) -> bool {
        !self.left && !self.right && !self.top && !self.bottom
    }

    /// 是否为对角（同时含一条横边与一条竖边）。
    #[must_use]
    pub(crate) const fn is_corner(self) -> bool {
        self.count() == 2
    }

    /// 参与缩放的边数（1 = 边缘，2 = 对角）。
    #[must_use]
    pub(crate) const fn count(self) -> u8 {
        (self.left as u8) + (self.right as u8) + (self.top as u8) + (self.bottom as u8)
    }

    /// 打包成 4 个 0/1 浮点（左 右 上 下），进独立 border pass 的 `BorderCb.b2`。
    ///
    /// 对角把手返回全 0：直边与对角的掩码必须在**源头**互斥。
    /// 如果这里把 `TOP_LEFT` 打成 `left|top`，着色器会同时画「到切点为止的直线」
    /// 和「沿圆角的弧」，两者在切点附近叠出一个亮斑（需求 F2.1 明确不要这个）。
    #[must_use]
    pub(crate) const fn to_shader_edges(self) -> [f32; 4] {
        if self.is_corner() {
            return [0.0; 4];
        }
        [
            self.left as i32 as f32,
            self.right as i32 as f32,
            self.top as i32 as f32,
            self.bottom as i32 as f32,
        ]
    }

    /// 打包成 4 个 0/1 浮点（左上 右上 左下 右下），进独立 border pass 的 `BorderCb.b3`。
    ///
    /// 只有对角把手才会产生非零值；边缘把手在着色器里走「直边 mask」，
    /// 不会沿圆角拐弯（需求 F2.1 vs F2.2 的分界）。
    #[must_use]
    pub(crate) const fn to_shader_corners(self) -> [f32; 4] {
        if !self.is_corner() {
            return [0.0; 4];
        }
        [
            (self.left && self.top) as i32 as f32,
            (self.right && self.top) as i32 as f32,
            (self.left && self.bottom) as i32 as f32,
            (self.right && self.bottom) as i32 as f32,
        ]
    }

    /// 映射到公共 API 的把手枚举（事件上抛用）。空集合返回 `None`。
    #[must_use]
    pub(crate) fn public(self) -> Option<crate::api::event::ResizeHandle> {
        use crate::api::event::ResizeHandle as H;
        match (self.left, self.right, self.top, self.bottom) {
            (true, false, false, false) => Some(H::Left),
            (false, true, false, false) => Some(H::Right),
            (false, false, true, false) => Some(H::Top),
            (false, false, false, true) => Some(H::Bottom),
            (true, false, true, false) => Some(H::TopLeft),
            (false, true, true, false) => Some(H::TopRight),
            (true, false, false, true) => Some(H::BottomLeft),
            (false, true, false, true) => Some(H::BottomRight),
            _ => None,
        }
    }

    /// 对应的系统光标 id（`WM_SETCURSOR` 用）。`None` = 保持默认。
    #[must_use]
    pub(crate) fn cursor_id(self) -> Option<u32> {
        // Win32 IDC_* 常量：见 platform::window。
        const IDC_SIZEWE: u32 = 32644;
        const IDC_SIZENS: u32 = 32645;
        const IDC_SIZENWSE: u32 = 32642;
        const IDC_SIZENESW: u32 = 32643;
        match (self.left, self.right, self.top, self.bottom) {
            (true, false, false, false) | (false, true, false, false) => Some(IDC_SIZEWE),
            (false, false, true, false) | (false, false, false, true) => Some(IDC_SIZENS),
            (true, false, true, false) | (false, true, false, true) => Some(IDC_SIZENWSE),
            (false, true, true, false) | (true, false, false, true) => Some(IDC_SIZENESW),
            _ => None,
        }
    }
}

/// 在块矩形上判定光标落在哪个把手。
///
/// `rect` = `[x, y, w, h]`（画布物理像素），`cursor` 同坐标系，`grip` = 命中带宽（物理像素）。
/// 返回 `None` 表示光标既不在块内也不在任何把手命中带里。
///
/// **先角后边**：角部命中盒（`grip × grip`）优先于两条边，
/// 否则在角上会同时命中横竖两条边、缩放方向抖动。
#[must_use]
pub(crate) fn hit_handle(rect: &[f32; 4], cursor: [f32; 2], grip: f32) -> Option<ResizeEdges> {
    let grip = grip.max(0.0);
    let (x, y, w, h) = (rect[0], rect[1], rect[2], rect[3]);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    // 命中带允许伸出块外 grip 像素（这样从块外也能抓住边缘），
    // 窗口区域（SetWindowRgn）必须外扩同样的量，否则拿不到鼠标消息。
    if cursor[0] < x - grip
        || cursor[0] > x + w + grip
        || cursor[1] < y - grip
        || cursor[1] > y + h + grip
    {
        return None;
    }
    let horizontal = if cursor[0] < x + grip {
        Some(ResizeEdges::LEFT)
    } else if cursor[0] > x + w - grip {
        Some(ResizeEdges::RIGHT)
    } else {
        None
    };
    let vertical = if cursor[1] < y + grip {
        Some(ResizeEdges::TOP)
    } else if cursor[1] > y + h - grip {
        Some(ResizeEdges::BOTTOM)
    } else {
        None
    };
    match (horizontal, vertical) {
        (Some(h), Some(v)) => Some(corner_of(h, v)),
        (Some(e), None) | (None, Some(e)) => Some(e),
        (None, None) => None,
    }
}

/// 横边 + 竖边 -> 对应的对角常量。
///
/// 单独抽出来是为了让四个角常量成为**唯一**的角定义来源：
/// 如果这里手写 `ResizeEdges { left: .., top: .. }`，常量就成了只给测试看的摆设。
const fn corner_of(h: ResizeEdges, v: ResizeEdges) -> ResizeEdges {
    match (h.left, v.top) {
        (true, true) => ResizeEdges::TOP_LEFT,
        (false, true) => ResizeEdges::TOP_RIGHT,
        (true, false) => ResizeEdges::BOTTOM_LEFT,
        (false, false) => ResizeEdges::BOTTOM_RIGHT,
    }
}

#[cfg(test)]
mod tests {
    use super::{hit_handle, ResizeEdges as E};

    const RECT: [f32; 4] = [100.0, 100.0, 400.0, 300.0];
    const GRIP: f32 = 10.0;

    #[test]
    fn center_is_not_a_handle() {
        assert_eq!(hit_handle(&RECT, [300.0, 250.0], GRIP), None);
    }

    #[test]
    fn all_four_edges_hit() {
        assert_eq!(hit_handle(&RECT, [104.0, 250.0], GRIP), Some(E::LEFT));
        assert_eq!(hit_handle(&RECT, [496.0, 250.0], GRIP), Some(E::RIGHT));
        assert_eq!(hit_handle(&RECT, [300.0, 104.0], GRIP), Some(E::TOP));
        assert_eq!(hit_handle(&RECT, [300.0, 396.0], GRIP), Some(E::BOTTOM));
    }

    #[test]
    fn all_four_corners_hit() {
        assert_eq!(hit_handle(&RECT, [104.0, 104.0], GRIP), Some(E::TOP_LEFT));
        assert_eq!(hit_handle(&RECT, [496.0, 104.0], GRIP), Some(E::TOP_RIGHT));
        assert_eq!(
            hit_handle(&RECT, [104.0, 396.0], GRIP),
            Some(E::BOTTOM_LEFT)
        );
        assert_eq!(
            hit_handle(&RECT, [496.0, 396.0], GRIP),
            Some(E::BOTTOM_RIGHT)
        );
    }

    #[test]
    fn corner_wins_over_edge() {
        // 同时落在左边带与上边带内 -> 必须判为对角，不能是单边
        let e = hit_handle(&RECT, [105.0, 105.0], GRIP).expect("角部应命中");
        assert!(e.is_corner(), "{e:?}");
        assert_eq!(e.count(), 2);
    }

    #[test]
    fn handle_band_extends_outside_the_block() {
        // 块外 grip/2 处仍能抓住边缘（窗口区域会同步外扩）
        assert_eq!(hit_handle(&RECT, [95.0, 250.0], GRIP), Some(E::LEFT));
        assert_eq!(hit_handle(&RECT, [505.0, 250.0], GRIP), Some(E::RIGHT));
    }

    #[test]
    fn outside_band_is_none() {
        assert_eq!(hit_handle(&RECT, [80.0, 250.0], GRIP), None);
        assert_eq!(hit_handle(&RECT, [520.0, 250.0], GRIP), None);
        assert_eq!(hit_handle(&RECT, [300.0, 420.0], GRIP), None);
    }

    #[test]
    fn degenerate_rect_never_hits() {
        assert_eq!(hit_handle(&[0.0, 0.0, 0.0, 100.0], [0.0, 50.0], GRIP), None);
        assert_eq!(
            hit_handle(&[0.0, 0.0, 100.0, -1.0], [50.0, 0.0], GRIP),
            None
        );
    }

    #[test]
    fn tiny_grip_still_hits_the_edge_pixel() {
        assert_eq!(hit_handle(&RECT, [100.0, 250.0], 0.5), Some(E::LEFT));
        assert_eq!(hit_handle(&RECT, [300.0, 250.0], 0.5), None);
    }

    #[test]
    fn zero_grip_degrades_to_nothing_but_never_panics() {
        // grip 会被 ResizeOptions::sanitize 夹到 >=2，这里只钉住退化行为：
        // 命中带宽度为 0 时任何点都不该命中（严格不等式，不产生 1px 抖动带）。
        assert_eq!(hit_handle(&RECT, [100.0, 250.0], 0.0), None);
        assert_eq!(hit_handle(&RECT, [300.0, 250.0], 0.0), None);
        assert_eq!(hit_handle(&RECT, [-1e9, -1e9], 0.0), None);
    }

    #[test]
    fn nan_cursor_does_not_hit() {
        assert_eq!(hit_handle(&RECT, [f32::NAN, 250.0], GRIP), None);
    }

    #[test]
    fn shader_packing_matches_semantics() {
        assert_eq!(E::LEFT.to_shader_edges(), [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(E::BOTTOM.to_shader_edges(), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(E::TOP_RIGHT.to_shader_corners(), [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(E::BOTTOM_LEFT.to_shader_corners(), [0.0, 0.0, 1.0, 0.0]);
        // 单边不产生任何角反馈（需求 F2.1：border 不沿圆角拐弯）
        assert_eq!(E::LEFT.to_shader_corners(), [0.0; 4]);
        assert_eq!(E::TOP.to_shader_corners(), [0.0; 4]);
        // 对角不产生直边反馈（需求 F2.2：只沿弧）
        assert_eq!(E::TOP_LEFT.to_shader_edges(), [0.0; 4]);
    }

    #[test]
    fn every_direction_has_a_cursor() {
        for e in [
            E::LEFT,
            E::RIGHT,
            E::TOP,
            E::BOTTOM,
            E::TOP_LEFT,
            E::TOP_RIGHT,
            E::BOTTOM_LEFT,
            E::BOTTOM_RIGHT,
        ] {
            assert!(e.cursor_id().is_some(), "{e:?} 缺少系统光标");
        }
        assert_eq!(E::NONE.cursor_id(), None);
    }

    #[test]
    fn none_is_empty_and_has_zero_count() {
        assert!(E::NONE.is_empty());
        assert_eq!(E::NONE.count(), 0);
        assert!(!E::NONE.is_corner());
    }
}
