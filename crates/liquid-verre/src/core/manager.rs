//! 玻璃块集合：增删改、z 序、命中测试。
//!
//! 纯逻辑（不碰 Win32 / GPU），因此全部可单测。
//! id 的分配不在这里：`api::handle::Shared` 在**调用方线程**分配 id，
//! 这样 `Engine::create_window` 能立刻把句柄还给上层，不必等渲染线程。

use crate::api::config::ZOrder;
use crate::core::block::Block;
use crate::core::handle::{hit_handle, ResizeEdges};

/// z 序归一化阈值：`send_to_back` 会往负方向走，长期运行可能溢出 i32。
/// 超过这个跨度就整体平移到 `0..n`（相对次序不变，渲染结果逐位一致）。
const Z_SPAN_LIMIT: i64 = 1 << 30;

/// 块集合。渲染顺序 = `z_order` 升序（在 `collect_monitor_blocks` 里排序）。
#[derive(Default)]
pub(crate) struct Manager {
    blocks: Vec<Block>,
}

impl Manager {
    /// 全部块（**未**按 z 排序，插入顺序）。
    #[must_use]
    pub(crate) fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// 可变借用全部块。
    ///
    /// 返回切片而不是 `&mut Vec`：调用方只能改字段，不能增删 —— 增删必须走
    /// `insert` / `remove`，否则 z 序归一化与样式缓存的失效逻辑会被绕过。
    pub(crate) fn blocks_mut(&mut self) -> &mut [Block] {
        &mut self.blocks
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.blocks.len()
    }

    /// 插入一个已构造好的块，返回其 id。
    ///
    /// z 序由调用方决定（`runtime::app::spawn_block` 会显式给值），这里只夹紧几何。
    pub(crate) fn insert(&mut self, mut b: Block) -> u32 {
        b.clamp();
        let id = b.id;
        self.blocks.push(b);
        id
    }

    pub(crate) fn remove(&mut self, id: u32) -> bool {
        let before = self.blocks.len();
        self.blocks.retain(|b| b.id != id);
        self.blocks.len() != before
    }

    pub(crate) fn clear(&mut self) {
        self.blocks.clear();
    }

    #[must_use]
    pub(crate) fn get(&self, id: u32) -> Option<&Block> {
        self.blocks.iter().find(|b| b.id == id)
    }

    pub(crate) fn get_mut(&mut self, id: u32) -> Option<&mut Block> {
        self.blocks.iter_mut().find(|b| b.id == id)
    }

    #[must_use]
    pub(crate) fn contains(&self, id: u32) -> bool {
        self.blocks.iter().any(|b| b.id == id)
    }

    /// 命中测试：返回最上层（z 最大）被命中的**可交互且可见**的块。
    ///
    /// 用块的精确矩形（不含 grip 外扩），因此点击阴影区域仍然是穿透的 —— 与重构前一致，
    /// 否则 `perf_probe.py` 的拖拽落点与 `verify_mouse_isolation.py` 的隔离证明都会变味。
    /// 边缘外侧的抓取由 [`Manager::hit_resize`] 负责。
    #[must_use]
    pub(crate) fn hit_test(&self, x: f32, y: f32) -> Option<u32> {
        self.blocks
            .iter()
            .filter(|b| b.interactive && b.visible)
            .filter(|b| x >= b.x && x <= b.x + b.width && y >= b.y && y <= b.y + b.height)
            .max_by_key(|b| b.z_order)
            .map(|b| b.id)
    }

    /// 缩放把手命中测试（需求 F2）。
    ///
    /// 命中带向外扩 `grip_px * scale`，因此可以在块外一点抓住边缘；
    /// 返回 `(块 id, 边集合)`。按 z 序自顶向下，第一个命中即返回。
    /// 关闭了 `resize.enabled`（或对应方向）的块不参与。
    #[must_use]
    pub(crate) fn hit_resize(&self, x: f32, y: f32) -> Option<(u32, ResizeEdges)> {
        // 每帧都会调用（主循环 + WM_NCHITTEST），因此**不得分配**：
        // 不收集候选再排序，而是单遍扫描记住 z 最大的命中（spec：热路径零堆分配）。
        let mut best: Option<(i32, u32, ResizeEdges)> = None;
        for b in self.blocks.iter() {
            if !(b.interactive && b.visible && b.resize.enabled) {
                continue;
            }
            let grip = b.resize.grip_px.max(0.0) * b.scale.max(1e-3);
            let rect = [b.x, b.y, b.width, b.height];
            let Some(edges) = hit_handle(&rect, [x, y], grip) else {
                continue;
            };
            // 被禁用的方向要剔除：只开 edges 时角部两条边都被去掉 -> 不响应；
            // 只开 corners 时单边不响应。
            // 判据走 ResizeOptions 的单一来源辅助函数，不在此处内联重写与/或逻辑
            let keep = if edges.is_corner() {
                b.resize.corners_active()
            } else {
                b.resize.edges_active()
            };
            if !keep {
                continue;
            }
            match best {
                Some((z, _, _)) if z >= b.z_order => {}
                _ => best = Some((b.z_order, b.id, edges)),
            }
        }
        best.map(|(_, id, edges)| (id, edges))
    }

    // ---------------- z 序 ----------------

    #[must_use]
    fn front_z(&self) -> i32 {
        self.blocks.iter().map(|b| b.z_order).max().unwrap_or(0)
    }

    #[must_use]
    fn back_z(&self) -> i32 {
        self.blocks.iter().map(|b| b.z_order).min().unwrap_or(0)
    }

    /// 抬到最前（需求 F4「点击到最前」）。返回 z 序是否真的变了，
    /// 调用方据此决定要不要触发重画 —— 已经在最前时返回 `false`，白重画一帧没意义。
    pub(crate) fn bring_to_front(&mut self, id: u32) -> bool {
        let max_z = self.front_z();
        let Some(b) = self.get_mut(id) else {
            return false;
        };
        if b.z_order >= max_z {
            return false;
        }
        b.z_order = max_z.saturating_add(1);
        self.normalize_z();
        true
    }

    /// 压到最后（需求 F4「一直被覆盖」）。
    pub(crate) fn send_to_back(&mut self, id: u32) -> bool {
        let min_z = self.back_z();
        let Some(b) = self.get_mut(id) else {
            return false;
        };
        if b.z_order <= min_z {
            return false;
        }
        b.z_order = min_z.saturating_sub(1);
        self.normalize_z();
        true
    }

    /// 按上层给的语义设置 z 序。返回是否变化。
    pub(crate) fn set_z(&mut self, id: u32, order: ZOrder) -> bool {
        let changed = match order {
            ZOrder::Front => self.bring_to_front(id),
            ZOrder::Back => self.send_to_back(id),
            ZOrder::Absolute(z) => match self.get_mut(id) {
                Some(b) if b.z_order != z => {
                    b.z_order = z;
                    true
                }
                _ => false,
            },
            ZOrder::Relative(d) => match self.get_mut(id) {
                Some(b) => {
                    let before = b.z_order;
                    b.z_order = b.z_order.saturating_add(d);
                    b.z_order != before
                }
                None => false,
            },
        };
        if changed {
            self.normalize_z();
        }
        changed
    }

    /// z 序跨度失控时整体平移到 `0..n`（保持相对次序，渲染结果不变）。
    fn normalize_z(&mut self) {
        let (Some(lo), Some(hi)) = (
            self.blocks.iter().map(|b| i64::from(b.z_order)).min(),
            self.blocks.iter().map(|b| i64::from(b.z_order)).max(),
        ) else {
            return;
        };
        if hi.saturating_sub(lo) < Z_SPAN_LIMIT && lo >= 0 && hi < Z_SPAN_LIMIT {
            return;
        }
        let mut order: Vec<(u32, i32)> = self.blocks.iter().map(|b| (b.id, b.z_order)).collect();
        order.sort_by_key(|(_, z)| *z);
        for (i, (id, _)) in order.into_iter().enumerate() {
            if let Some(b) = self.blocks.iter_mut().find(|b| b.id == id) {
                b.z_order = i as i32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Manager;
    use crate::api::config::{GlassStyle, ResizeOptions, ZOrder};
    use crate::core::block::Block;
    use crate::core::handle::ResizeEdges;

    /// 测试用：直接插一块（生产路径走 `runtime::app::spawn_block`）。
    fn add(m: &mut Manager, id: u32, x: f32, y: f32, w: f32, h: f32) -> u32 {
        let mut b = Block::new(id, x, y, w, h);
        b.z_order = id as i32;
        m.insert(b)
    }

    /// z 升序的 id 列表（= 渲染顺序）。
    fn order(m: &Manager) -> Vec<u32> {
        let mut v: Vec<(i32, u32)> = m.blocks().iter().map(|b| (b.z_order, b.id)).collect();
        v.sort_unstable();
        v.into_iter().map(|(_, id)| id).collect()
    }

    /// n 块互相重叠的玻璃（z 依次递增）。
    fn mgr_with(n: usize) -> Manager {
        let mut m = Manager::default();
        for i in 0..n {
            add(
                &mut m,
                (i + 1) as u32,
                50.0 + i as f32 * 10.0,
                50.0,
                300.0,
                200.0,
            );
        }
        m
    }

    #[test]
    fn ids_are_kept_as_given() {
        let m = mgr_with(5);
        let ids: Vec<u32> = m.blocks().iter().map(|b| b.id).collect();
        assert_eq!(ids, vec![1, 2, 3, 4, 5]);
        assert_eq!(m.len(), 5);
        assert!(m.contains(3) && !m.contains(99));
    }

    #[test]
    fn hit_test_returns_topmost() {
        let m = mgr_with(3);
        assert_eq!(m.hit_test(120.0, 120.0), Some(3));
    }

    #[test]
    fn hit_test_ignores_outside_invisible_and_noninteractive() {
        let mut m = mgr_with(2);
        assert_eq!(m.hit_test(5000.0, 5000.0), None);
        m.get_mut(2).unwrap().interactive = false;
        assert_eq!(m.hit_test(120.0, 120.0), Some(1));
        m.get_mut(1).unwrap().visible = false;
        assert_eq!(m.hit_test(120.0, 120.0), None, "隐藏块既不命中也不参与缩放");
        assert_eq!(m.hit_resize(104.0, 104.0), None);
    }

    #[test]
    fn bring_to_front_and_send_to_back_change_order() {
        let mut m = mgr_with(3);
        assert_eq!(order(&m), vec![1, 2, 3]);
        assert!(m.bring_to_front(1));
        assert_eq!(order(&m), vec![2, 3, 1]);
        assert!(m.send_to_back(1));
        assert_eq!(order(&m), vec![1, 2, 3]);
        // 幂等：已经在最前 / 最后时返回 false（调用方据此决定要不要重画）
        assert!(!m.bring_to_front(3));
        assert!(!m.send_to_back(1));
    }

    #[test]
    fn set_z_supports_absolute_and_relative() {
        let mut m = mgr_with(3);
        assert!(m.set_z(1, ZOrder::Absolute(99)));
        assert_eq!(order(&m), vec![2, 3, 1]);
        assert!(m.set_z(1, ZOrder::Relative(-1000)));
        assert_eq!(order(&m), vec![1, 2, 3]);
        assert!(m.set_z(2, ZOrder::Front));
        assert_eq!(order(&m), vec![1, 3, 2]);
        let cur = m.get(2).unwrap().z_order;
        assert!(
            !m.set_z(2, ZOrder::Absolute(cur)),
            "值没变就不该报告 changed"
        );
        assert!(
            !m.set_z(2, ZOrder::Relative(0)),
            "偏移 0 也不该报告 changed"
        );
    }

    #[test]
    fn z_never_overflows_after_many_moves() {
        let mut m = mgr_with(3);
        for _ in 0..2000 {
            m.bring_to_front(1);
            m.send_to_back(1);
        }
        for b in m.blocks() {
            assert!(b.z_order.abs() < (1 << 20), "z={}", b.z_order);
        }
        assert_eq!(order(&m).len(), 3);
    }

    #[test]
    fn z_ops_on_missing_id_are_false() {
        let mut m = mgr_with(1);
        assert!(!m.set_z(999, ZOrder::Front));
        assert!(!m.set_z(999, ZOrder::Absolute(5)));
        assert!(!m.set_z(999, ZOrder::Relative(5)));
        assert!(!m.bring_to_front(999));
        assert!(!m.send_to_back(999));
    }

    #[test]
    fn resize_hit_respects_per_block_grip() {
        let mut m = Manager::default();
        let id = add(&mut m, 1, 100.0, 100.0, 400.0, 300.0);
        // 默认 grip 10：块内 4px 处命中左边
        assert_eq!(m.hit_resize(104.0, 250.0), Some((id, ResizeEdges::LEFT)));
        // 块外 20px 处不命中
        assert_eq!(m.hit_resize(80.0, 250.0), None);
        m.get_mut(id).unwrap().resize.grip_px = 40.0;
        assert_eq!(m.hit_resize(80.0, 250.0), Some((id, ResizeEdges::LEFT)));
    }

    #[test]
    fn resize_hit_honours_edge_and_corner_toggles() {
        let mut m = Manager::default();
        let id = add(&mut m, 1, 100.0, 100.0, 400.0, 300.0);
        // 只开对角：单边不响应，角部仍响应
        m.get_mut(id).unwrap().resize = ResizeOptions {
            edges: false,
            corners: true,
            ..Default::default()
        };
        assert_eq!(m.hit_resize(104.0, 250.0), None);
        assert_eq!(
            m.hit_resize(104.0, 104.0),
            Some((id, ResizeEdges::TOP_LEFT))
        );
        // 只开边缘：角部不响应
        m.get_mut(id).unwrap().resize = ResizeOptions {
            edges: true,
            corners: false,
            ..Default::default()
        };
        assert_eq!(m.hit_resize(104.0, 104.0), None);
        assert_eq!(m.hit_resize(104.0, 250.0), Some((id, ResizeEdges::LEFT)));
        // 总开关关掉：全部不响应
        m.get_mut(id).unwrap().resize.enabled = false;
        assert_eq!(m.hit_resize(104.0, 250.0), None);
        assert_eq!(m.hit_resize(104.0, 104.0), None);
    }

    #[test]
    fn resize_hit_prefers_topmost_block() {
        // mgr_with: #1 在 (50,50) 300x200，#2 在 (60,50) 300x200（z 更高）。
        // (64,150) 落在 #2 的左边带内（grip=10），而 #1 的左边带是 x<60 —— 不命中。
        let m = mgr_with(2);
        let top = m.blocks()[1].id;
        let hit = m.hit_resize(64.0, 150.0).expect("应命中最上层块的左边");
        assert_eq!(hit.0, top);
        assert_eq!(hit.1, ResizeEdges::LEFT);
        // 两块重叠区域的角上，也必须判给 z 更大的那块
        let corner = m.hit_resize(64.0, 54.0).expect("应命中左上角");
        assert_eq!(corner.0, top);
        assert_eq!(corner.1, ResizeEdges::TOP_LEFT);
    }

    #[test]
    fn resize_hit_scales_grip_by_dpi() {
        let mut m = Manager::default();
        let id = add(&mut m, 1, 100.0, 100.0, 400.0, 300.0);
        m.get_mut(id).unwrap().resize.grip_px = 10.0;
        // 100% DPI：块外 15px 抓不到
        assert_eq!(m.hit_resize(85.0, 250.0), None);
        // 200% DPI：grip 变成 20 物理 px，同一个点就抓得到了
        m.get_mut(id).unwrap().scale = 2.0;
        assert_eq!(m.hit_resize(85.0, 250.0), Some((id, ResizeEdges::LEFT)));
    }

    #[test]
    fn remove_and_clear() {
        let mut m = mgr_with(3);
        assert!(m.remove(2));
        assert!(!m.remove(2));
        assert_eq!(m.len(), 2);
        m.clear();
        assert_eq!(m.len(), 0);
        assert_eq!(m.hit_test(120.0, 120.0), None);
    }

    #[test]
    fn insert_clamps_geometry() {
        let mut m = Manager::default();
        let mut b = Block::new(9, 0.0, 0.0, 1e9, 1e9);
        b.style = GlassStyle::transparent(10.0, 0.5, [1.0; 4]);
        m.insert(b);
        assert!(m.get(9).unwrap().width <= crate::core::block::MAX_SIZE);
    }
}
