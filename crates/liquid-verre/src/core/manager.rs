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

/// 该点被哪一层「本体」压着：覆盖该点的**可见**块里最大的 `z_order`；无覆盖返回 `i32::MIN`。
///
/// 0.5.3 #9 的判据本体。口径与 [`Manager::hit_test`] **逐字一致**（精确矩形
/// `x..x+width`、`y..y+height`，不含阴影外扩、不含 grip 外扩），这样
/// 「绘制层序」`core::draw::border_emit_plan`、「本体命中」`hit_test`、
/// 「把手命中」`hit_resize` 三者共用同一套 z 语义 —— 这正是 BASELINE 不变量 13
/// （绘制的层序与命中的层序必须共用同一套 z 判据）的落地方式。
///
/// 两条刻意的取舍（清单 §3.3 细则 2/3）：
/// - `visible == false` 的块**不参与**遮挡：隐藏的块没有像素；
/// - `interactive == false` 的块**参与**遮挡：它的玻璃确实盖在上面（锁定态、
///   `set_interactive(false)` 都是这种）。遮挡是视觉事实，与可交互性无关。
///
/// 零分配、O(n)，n = 块数（宿主典型 4~20，极限 32）。
#[must_use]
pub(crate) fn topmost_body_z_at(blocks: &[Block], x: f32, y: f32) -> i32 {
    let mut top = i32::MIN;
    for b in blocks {
        if b.visible && x >= b.x && x <= b.x + b.width && y >= b.y && y <= b.y + b.height {
            // `max` 而不是嵌套 if：clippy::collapsible_if 会拦，而这里也确实没有别的分支。
            top = top.max(b.z_order);
        }
    }
    top
}

/// 把手候选在该点是否**够得着**：`candidate_z >= top_body_z`。
///
/// **严格小于才抑制**（细则 1）：z 相等必须放行，否则块自己机身边缘内侧那一圈
/// 会被自己判死 —— #9 修完立刻多出「边缘内侧 1px 抓不动」的新缺陷。
/// 无覆盖时 `top_body_z = i32::MIN`，全部候选自然放行。
#[must_use]
pub(crate) fn grip_reachable(candidate_z: i32, top_body_z: i32) -> bool {
    candidate_z >= top_body_z
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
    ///
    /// ⚠ **0.5.3 #9**：候选还必须**没有被更上层块的本体压在这个点上**。旧实现只在
    /// 把手候选之间比 z 序，而上层块 B 在该点根本不是候选（光标在 B 机身内部、
    /// 不在 B 的边缘带）⇒ B 连参与比较的机会都没有，下层块 A 作为唯一候选当选：
    /// hover / 反馈 border / 手型光标全落在被 B 盖住的 A 边缘上，点下去又拉不动
    /// （观感与行为割裂）。0.5.2 #1 只修了**绘制**层序，没修**命中**层序 ——
    /// 半边修复就是本次缺陷的根因，见 BASELINE 不变量 13。
    ///
    /// 四个调用点（`runtime::mod` 每帧 hover、`WM_NCHITTEST`、`dispatch_pointer`、
    /// 以及走 `hit_test` 的 `forward_move_to_sink`）**共用这一处判据**，改一次到底，
    /// 不会互相漂移（#9 之前「Move 归上层块、亮边归下层块」的自相矛盾正是漂移）。
    #[must_use]
    pub(crate) fn hit_resize(&self, x: f32, y: f32) -> Option<(u32, ResizeEdges)> {
        // 每帧都会调用（主循环 + WM_NCHITTEST），因此**不得分配**：
        // 不收集候选再排序，而是单遍扫描记住 z 最大的命中（spec：热路径零堆分配）。
        //
        // 0.5.3 #9 因此写成**两趟 O(2n) 无分配扫描**。不要改成「收集候选再排序」——
        // 那正是 0.5.2 里 `border_emit_plan` 每帧新建 Vec 把 `own_us` 从 14 抬到 142
        // 的同款坑（gate_perf.py 的 transparent WARN 抓到的就是它）。
        let top_body_z = topmost_body_z_at(&self.blocks, x, y);
        let mut best: Option<(i32, u32, ResizeEdges)> = None;
        for b in self.blocks.iter() {
            if !(b.interactive && b.visible && b.resize.enabled) {
                continue;
            }
            // 本条修复的唯一新增判据：被更上层块的本体压住的把手，直接丢弃
            // （细则 5：不降级成「该点命中上层块本体」，那点归 hit_test 判）。
            if !grip_reachable(b.z_order, top_body_z) {
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
    use super::{grip_reachable, topmost_body_z_at, Manager};
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

    // ---- 0.5.3 #9：把手命中必须让位更上层块的本体 ----
    //
    // 场景统一：A = id 1，z=1，(100,100) 400x300；B = id 2，z=2，(60,150) 300x200。
    // B 的**机身**盖住 A 左边缘的中段（x=104 落在 60..360 内，y=250 落在 150..350 内），
    // 而 B 自己的把手带（grip=10 ⇒ x∈[50,70]）不含 x=104 ⇒ B 在该点**不是候选**。
    // 这正是主诉的形状：旧判据里 B 连参与比较的机会都没有。
    fn pair() -> Manager {
        let mut m = Manager::default();
        add(&mut m, 1, 100.0, 100.0, 400.0, 300.0);
        add(&mut m, 2, 60.0, 150.0, 300.0, 200.0);
        m
    }

    /// 主判据：A 左边缘带里的点被上层块 B 的机身压住 ⇒ **绝不许返回 A**。
    ///
    /// 判别力（清单 §4 要求）：把 `hit_resize` 里那句
    /// `if !grip_reachable(b.z_order, top_body_z) { continue; }` 注释掉，
    /// 本测试立刻红，而其余 5 条仍绿 —— 这条就是 #9 的全部。
    #[test]
    fn grip_under_higher_body_does_not_hover() {
        let m = pair();
        let hit = m.hit_resize(104.0, 250.0);
        assert!(
            hit.map_or(true, |(id, _)| id != 1),
            "被上层块机身压住的边缘不许把 hover 判给下层块（实得 {hit:?}）"
        );
        // 同一点的本体归属仍然是 B（上层块机身）⇒ 拖拽归 B，与亮边不再矛盾。
        assert_eq!(m.hit_test(104.0, 250.0), Some(2));
    }

    /// 反向对照（防「修过头」把整条边缘带关掉）：同一个边缘、点在 B 矩形之外 ⇒ 照常命中 A。
    #[test]
    fn grip_outside_higher_body_still_hovers() {
        let m = pair();
        // y=120 在 B（150..350）之外，x=104 仍在 A 的左边缘带内（grip=10）。
        assert_eq!(
            m.hit_resize(104.0, 120.0),
            Some((1, ResizeEdges::LEFT)),
            "没被压住的那段边缘必须照常亮"
        );
    }

    /// 细则 1：**相等不抑制**。只有 A 一块时，点 A 边缘内侧 1px 仍要抓到 A，
    /// 否则 #9 修完立刻新增「边缘内侧 1px 抓不动」的缺陷。
    #[test]
    fn grip_inside_own_body_is_not_self_occluded() {
        let mut m = Manager::default();
        let id = add(&mut m, 1, 100.0, 100.0, 400.0, 300.0);
        // 该点被 A 自己的本体覆盖 ⇒ top_body_z == A.z == 候选 z ⇒ 放行。
        assert_eq!(
            topmost_body_z_at(m.blocks(), 101.0, 250.0),
            m.blocks()[0].z_order
        );
        assert_eq!(m.hit_resize(101.0, 250.0), Some((id, ResizeEdges::LEFT)));
        // 再往外（本体之外、把手带之内）同样命中：那里没有任何本体压着。
        assert_eq!(m.hit_resize(96.0, 250.0), Some((id, ResizeEdges::LEFT)));
    }

    /// 细则 2：`visible = false` 的上层块没有像素，不许参与遮挡。
    #[test]
    fn hidden_higher_block_does_not_occlude_grip() {
        let mut m = pair();
        m.get_mut(2).unwrap().visible = false;
        assert_eq!(
            m.hit_resize(104.0, 250.0),
            Some((1, ResizeEdges::LEFT)),
            "隐藏块不该吃掉下层块未被遮挡的边缘"
        );
    }

    /// 细则 3：`interactive = false` 但**可见**的上层块照样参与遮挡
    /// （锁定态的玻璃确实盖在上面，遮挡是视觉事实，与可交互性无关）。
    #[test]
    fn non_interactive_higher_block_still_occludes_grip() {
        let mut m = pair();
        m.get_mut(2).unwrap().interactive = false;
        let hit = m.hit_resize(104.0, 250.0);
        assert!(
            hit.map_or(true, |(id, _)| id != 1),
            "不可交互但可见的上层块必须参与遮挡（实得 {hit:?}）"
        );
        // 本体命中这条**不**跟着遮挡走：`interactive = false` 的语义就是「画出来但不接鼠标」，
        // 所以该点的按压仍归可交互的 A（拖拽移动 A）。两条判据各自正确：
        // **亮边**看视觉遮挡（B 的玻璃确实盖住 A 的边缘），**归属**看可交互性（B 不吃鼠标）。
        assert_eq!(m.hit_test(104.0, 250.0), Some(1));
    }

    /// 判据本体的真值表：多块叠加取最大 z、无覆盖取 i32::MIN、相等放行。
    #[test]
    fn topmost_body_z_at_is_single_source_of_truth() {
        let m = pair();
        let blocks = m.blocks();
        // 被 B（z=2）机身压住的点
        assert_eq!(topmost_body_z_at(blocks, 104.0, 250.0), 2);
        // 只在 A 里的点（B 的右边界是 60+300=360，取 400 才落在 B 之外）
        assert_eq!(topmost_body_z_at(blocks, 400.0, 250.0), 1);
        // 谁都不在的点
        assert_eq!(topmost_body_z_at(blocks, 10.0, 10.0), i32::MIN);
        // 边界闭区间口径必须与 hit_test 一致（x == x+width 也算覆盖）
        assert_eq!(topmost_body_z_at(blocks, 100.0 + 400.0, 250.0), 1);
        // grip_reachable 真值表
        assert!(grip_reachable(5, i32::MIN), "无覆盖 ⇒ 全部放行");
        assert!(grip_reachable(5, 5), "相等不抑制（细则 1）");
        assert!(!grip_reachable(4, 5), "严格小于才抑制");
        assert!(grip_reachable(6, 5), "更上层候选自己就是最上层 ⇒ 放行");
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
