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

/// `rect_fully_covered` 的碎片数上限：超过就**保守**判「未遮挡」。
///
/// 差集产生的碎片两两不相交，且每个顶点都落在 `target` 或某个 cover 的边上，
/// 因此碎片数的上限是矩形排列的复杂度 O(m^2)，**不会指数爆炸**。设这个帽子是为了
/// 把最坏情况的成本钉死：cover 来自 `EnumWindows`，而桌面上的窗口数没有上界，
/// 万一某台机器上有上百个互相交叠的窗口，8 Hz 的轮询也不该退化成长时间计算。
/// 256 已远超真实量级 —— 调用方只把「与本屏块外接盒相交的不透明窗口」传进来，
/// 默认 `AlwaysOnTop` 层级下这个集合恒为空。
const OPEN_RECT_CAP: usize = 256;

/// 矩形是否退化（半开区间下覆盖 0 个像素）。
///
/// 与 `rect_area` 的区别：它只回答「有没有像素」，不做 i64 乘法，
/// 因此在「零面积窗口直接跳过」这类判定里更便宜。单一来源，别处不要再手写
/// `x1 <= x0 || y1 <= y0`。
#[must_use]
pub fn rect_is_empty(r: [i32; 4]) -> bool {
    rect_is_empty_i64([
        i64::from(r[0]),
        i64::from(r[1]),
        i64::from(r[2]),
        i64::from(r[3]),
    ])
}

/// [`rect_is_empty`] 的宽整数版 —— 「矩形退化」这个判据**全仓只写在这一处**。
///
/// 为什么要有 i64 版、而不是让调用方先窄化回 `[i32; 4]` 再调上面那个：
/// `platform::occlusion` 的 `EnumWindows` 回调把 Win32 给的窗口坐标 widen 到 i64 之后
/// **就不再窄化**，那是「回调结构上不可能 panic」纪律的一部分（panic 穿 FFI 边界 +
/// release profile 的 `panic = "abort"` ⇒ 整个渲染进程直接没了，装 `catch_unwind` 也拦不住）。
/// 让它为了复用一个判据而退回 i32，等于用纪律换「看起来没有重复」，方向错了。
///
/// 顺带把一件容易记错的事写清楚：这个判据**只有比较、没有算术**，所以 widen 在这里
/// **不是**为了防溢出（比较根本不会溢出），只是让调用方能一路保持 i64、中途不做窄化。
/// 真正必须 widen 的是加减法（见 `runtime::occlusion::to_abs`）。
#[must_use]
pub fn rect_is_empty_i64(r: [i64; 4]) -> bool {
    // 解构而不是下标：与本文件其余判据同一风格，也让「不可能越界」成为结构性质。
    let [x0, y0, x1, y1] = r;
    x1 <= x0 || y1 <= y0
}

/// 只在非退化时入列。
///
/// 差集必然产出空条带（cover 在某条轴上完整跨过 `r` 时，上/下 或 左/右 那两条
/// 厚度为 0），放进列表只会让碎片数虚涨、白烧一轮比较。
fn push_nonempty(out: &mut Vec<[i32; 4]>, r: [i32; 4]) {
    if !rect_is_empty(r) {
        out.push(r);
    }
}

/// `target` 是否被 `covers` 的并集**完整**覆盖（`[x0, y0, x1, y1]`，
/// **半开区间**，虚拟屏绝对物理像素）。
///
/// 半开语义是判据正确的关键：`cover.x1 == target.x1`（右边界恰好接触）必须算覆盖。
/// 差 1 px 的后果不是「略微不准」，而是「无边框全屏窗口压住整块玻璃」**永远**判不出
/// 遮挡 —— 遮挡剔除会静默失效，而所有单测照样绿。
///
/// # 语义（每条都有对应单测）
/// - 空 `covers` → `false`；退化的（0 面积）cover 直接忽略，它覆盖 0 个像素。
/// - **零面积 `target`** → `true`：没有像素需要覆盖。方向是安全的 ——
///   它不会让任何一个**可见**像素失去触发源。
/// - 碎片数超过 `OPEN_RECT_CAP` → `false`：**保守退出**，判「未遮挡」
///   ⇒ 错判方向永远是「多渲染」而不是「少渲染」。
///
/// # 算法与成本
/// 分割法：维护「尚未被覆盖」的矩形列表（初值 `[target]`），对每个 cover 做差集
/// （一个矩形减一个矩形最多产生上/下/左/右 4 条残差），列表清空 ⇒ 完整覆盖。
/// 复杂度 O(m * k)，m = cover 数、k = 当前碎片数。
///
/// **为什么这里允许堆分配**：只在 8 Hz 的遮挡轮询里调用（`runtime::occlusion::update`），
/// **不在每帧热路径上**。spec 的「热路径零堆分配」约束针对的是每帧的 prep / render
/// （20 块 × 60~144 fps = 每秒上千次），而这里是每秒 8 次 × ≤20 块 × 2 个 `Vec`。
/// 换来的是：纯函数、可单测、调用方不必持有并复用 scratch 缓冲区
/// （少一处「谁负责 clear」的隐性契约）。
#[must_use]
pub fn rect_fully_covered(target: [i32; 4], covers: &[[i32; 4]]) -> bool {
    // 零面积 target：没有像素需要覆盖。
    if rect_is_empty(target) {
        return true;
    }
    // 快路径：单个 cover 就能盖住。真实场景里这是绝大多数（一个无边框全屏窗口、
    // 或一条铺满的任务栏压住整块玻璃），不必进分割循环，也不必分配。
    if covers.iter().any(|c| rect_contains(c, &target)) {
        return true;
    }

    let mut open: Vec<[i32; 4]> = Vec::with_capacity(4);
    open.push(target);
    let mut next: Vec<[i32; 4]> = Vec::with_capacity(8);
    for c in covers {
        if rect_is_empty(*c) {
            continue;
        }
        // 双缓冲而不是原地 swap_remove：原地替换会把**本轮新生成**的残差
        // 挪进还没扫过的下标，导致某个原有碎片被跳过（少减一次 = 假阳性遮挡）。
        next.clear();
        for &r in &open {
            // `rects_hit` 是开区间：仅边界相接不算相交 ⇒ 残差原样保留。
            if !rects_hit(&r, c) {
                next.push(r);
                continue;
            }
            // 交集（`rects_hit` 为真已保证它在两条轴上都非退化）
            let ix0 = r[0].max(c[0]);
            let iy0 = r[1].max(c[1]);
            let ix1 = r[2].min(c[2]);
            let iy1 = r[3].min(c[3]);
            // r \ c 恰好等于这四条残差的并集，且它们两两不相交
            push_nonempty(&mut next, [r[0], r[1], r[2], iy0]); // 上
            push_nonempty(&mut next, [r[0], iy1, r[2], r[3]]); // 下
            push_nonempty(&mut next, [r[0], iy0, ix0, iy1]); // 左
            push_nonempty(&mut next, [ix1, iy0, r[2], iy1]); // 右
        }
        if next.len() > OPEN_RECT_CAP {
            return false;
        }
        std::mem::swap(&mut open, &mut next);
        if open.is_empty() {
            return true;
        }
    }
    // 走到这里 `open` 必非空（清空的情形已在循环内提前 return）：还有像素露在外面。
    open.is_empty()
}

/// 点到显示器的归属：到各屏中心距离最近的下标。
///
/// 纯算术（0 unsafe、0 依赖），不读环境变量。
/// 原在 platform/display.rs，现沉入 core 以获得单测判别力
/// （design #4：归屏算法的正确性不应依赖真实显示器枚举）。
/// 空数组返回 0（与原入口一致）。
#[must_use]
pub fn index_at(rects: &[([i32; 2], [i32; 2])], x: f32, y: f32) -> usize {
    if rects.is_empty() {
        return 0;
    }
    let mut best = 0;
    let mut best_d = f32::MAX;
    for (i, &(origin, size)) in rects.iter().enumerate() {
        let w = size[0] as f32;
        let h = size[1] as f32;
        let cx = (x - (origin[0] as f32 + w * 0.5)).abs();
        let cy = (y - (origin[1] as f32 + h * 0.5)).abs();
        let d = cx * cx + cy * cy;
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covered_by_a_single_larger_rect() {
        // cover 比 target 大（含向 target 外延伸）⇒ 覆盖
        assert!(rect_fully_covered([0, 0, 10, 10], &[[-5, -5, 20, 20]]));
        assert!(rect_fully_covered([0, 0, 10, 10], &[[0, 0, 10, 10]]));
        assert!(rect_fully_covered(
            [0, 0, 10, 10],
            &[[-10, -10, 10, 5], [0, 5, 20, 20]]
        ));
    }

    #[test]
    fn empty_or_degenerate_covers_never_cover_a_real_target() {
        assert!(!rect_fully_covered([0, 0, 10, 10], &[]));
        // 只有 0 面积 cover 等价于空集合（它覆盖 0 个像素）
        assert!(!rect_fully_covered(
            [0, 0, 10, 10],
            &[[3, 3, 3, 9], [0, 0, 0, 0]]
        ));
    }

    #[test]
    fn zero_area_target_counts_as_covered() {
        // 没有像素需要覆盖 ⇒ true。方向安全：不会让任何**可见**像素失去触发源。
        assert!(rect_fully_covered([5, 5, 5, 5], &[]));
        assert!(rect_fully_covered([0, 0, 10, 0], &[]));
        assert!(rect_fully_covered([7, 0, 3, 9], &[]));
    }

    #[test]
    fn adjacent_edges_count_as_covered() {
        // 半开区间：cover 的边界恰好等于 target 的边界必须算覆盖。
        // 差 1 px 会让「全屏窗口压住整块玻璃」永远判不出遮挡，而单测照样绿。
        assert!(rect_fully_covered(
            [0, 0, 10, 10],
            &[[0, 0, 10, 5], [0, 5, 10, 10]]
        ));
        assert!(rect_fully_covered(
            [0, 0, 10, 10],
            &[[0, 0, 4, 10], [4, 0, 10, 10]]
        ));
        assert!(rect_fully_covered(
            [0, 0, 10, 10],
            &[[0, 0, 10, 10], [10, 0, 20, 10]]
        ));
    }

    #[test]
    fn a_one_pixel_gap_is_not_covered() {
        // 反向也必须精确：漏一行像素就不许判遮挡（宁可多渲染，不可少渲染）
        assert!(!rect_fully_covered(
            [0, 0, 10, 10],
            &[[0, 0, 10, 5], [0, 6, 10, 10]]
        ));
        assert!(!rect_fully_covered(
            [0, 0, 10, 10],
            &[[0, 0, 9, 10], [9, 0, 10, 9]]
        ));
    }

    #[test]
    fn partial_cover_is_false() {
        assert!(!rect_fully_covered([0, 0, 10, 10], &[[0, 0, 10, 5]]));
        assert!(!rect_fully_covered([0, 0, 10, 10], &[[0, 0, 5, 10]]));
        assert!(!rect_fully_covered([0, 0, 10, 10], &[[2, 2, 8, 8]]));
    }

    #[test]
    fn l_shape_and_quadrants_need_the_splitting_path() {
        // 没有任何单个 cover 能盖住 target ⇒ 快路径必然落空，真的走分割法
        assert!(rect_fully_covered(
            [0, 0, 10, 10],
            &[[0, 0, 10, 4], [0, 4, 4, 10], [4, 4, 10, 10]]
        ));
        // L 形少一块就不算
        assert!(!rect_fully_covered(
            [0, 0, 10, 10],
            &[[0, 0, 10, 4], [0, 4, 4, 10]]
        ));
        // 四象限拼接
        assert!(rect_fully_covered(
            [0, 0, 10, 10],
            &[[0, 0, 5, 5], [5, 0, 10, 5], [0, 5, 5, 10], [5, 5, 10, 10]]
        ));
    }

    #[test]
    fn negative_coordinates_work() {
        // 副屏排在主屏左侧时 origin 是负的，判据不能只在第一象限成立
        assert!(rect_fully_covered(
            [-100, -50, -20, -10],
            &[[-100, -50, -60, -10], [-60, -50, -20, -10]]
        ));
        // 只盖住左半截（cover 右边界 -50 < target 右边界 -20）⇒ 不算遮挡。
        // 注意别把 [-200,-200,0,0] 当反例：它的右/下边界 0 已经越过 target 的
        // -20/-10，是**完整包住**（负坐标下「更大的数」仍然是更靠右下）。
        assert!(!rect_fully_covered(
            [-100, -50, -20, -10],
            &[[-200, -200, -50, 0]]
        ));
        assert!(rect_fully_covered([-10, -10, 0, 0], &[[-10, -10, 0, 0]]));
    }

    #[test]
    fn cover_order_does_not_matter() {
        // 分割法对 cover 顺序不敏感（差集满足交换律），乱序不许给出不同结论
        assert!(rect_fully_covered(
            [0, 0, 10, 10],
            &[[0, 0, 10, 4], [4, 4, 10, 10], [0, 4, 4, 10]]
        ));
        assert!(rect_fully_covered(
            [0, 0, 10, 10],
            &[[0, 4, 4, 10], [4, 4, 10, 10], [0, 0, 10, 4]]
        ));
    }

    #[test]
    fn fullscreen_window_over_a_real_glass_block() {
        // 真实量级：1920x1080 主屏上一块 560x360 的玻璃（已含阴影外扩），
        // 被一个无边框铺满屏的窗口压住 —— 这正是 A/B1 要吃掉的场景。
        let block = [320, 200, 880, 560];
        assert!(rect_fully_covered(block, &[[0, 0, 1920, 1080]]));
        // 只盖住左半边（另一个 800px 宽的窗口）⇒ 不算遮挡，B1 不给任何收益
        assert!(!rect_fully_covered(block, &[[0, 0, 800, 1080]]));
    }

    /// 确定性伪随机（xorshift64）：单测不许引第三方 `rand`，也不许每次跑出不同结果
    /// （不可复现的失败等于没有测试）。
    fn next_rand(seed: &mut u64) -> u64 {
        let mut x = *seed;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *seed = x;
        x
    }

    /// 随机矩形，坐标落在 `[-4, 11]`：**故意跨过 0**，因为副屏排在主屏左侧时
    /// `origin` 是负的，只在第一象限成立的判据是假绿。
    fn rand_rect(seed: &mut u64) -> [i32; 4] {
        let c = |seed: &mut u64| (next_rand(seed) % 16) as i32 - 4;
        let (a, b) = (c(seed), c(seed));
        let (d, e) = (c(seed), c(seed));
        [a.min(b), d.min(e), a.max(b), d.max(e)]
    }

    #[test]
    fn fully_covered_matches_a_brute_force_pixel_oracle() {
        // 分割法差集是全仓最容易藏 off-by-one 的地方（半开区间 + 负坐标 +
        // 退化残差 + cover 顺序），所以拿「逐像素标记」这个笨办法当 oracle 对撞。
        // 4000 组随机输入、每组 ≤3 个 cover：碎片上限 4^3 = 64，远够不到
        // OPEN_RECT_CAP，因此不会踩到「保守退出」分支而假失败。
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        for case in 0..4000u32 {
            let target = rand_rect(&mut seed);
            let covers: Vec<[i32; 4]> = (0..next_rand(&mut seed) % 4)
                .map(|_| rand_rect(&mut seed))
                .collect();

            // oracle：半开区间 [x0,x1) x [y0,y1) 逐像素问「有没有 cover 盖住我」。
            // 零面积 target 的循环体一次都不执行 ⇒ uncovered=0 ⇒ 期望 true，
            // 与 rect_fully_covered 的「零面积算已覆盖」约定一致。
            let mut uncovered = 0usize;
            for y in target[1]..target[3] {
                for x in target[0]..target[2] {
                    let hit = covers
                        .iter()
                        .any(|c| x >= c[0] && x < c[2] && y >= c[1] && y < c[3]);
                    if !hit {
                        uncovered += 1;
                    }
                }
            }
            assert_eq!(
                rect_fully_covered(target, &covers),
                uncovered == 0,
                "case {} target={:?} covers={:?} uncovered_px={}",
                case,
                target,
                covers,
                uncovered
            );
        }
    }

    #[test]
    fn rect_is_empty_matches_half_open_convention() {
        assert!(rect_is_empty([5, 5, 5, 9]));
        assert!(rect_is_empty([5, 5, 9, 5]));
        assert!(rect_is_empty([9, 9, 5, 5]));
        assert!(!rect_is_empty([5, 5, 6, 6]));
        // 与 rect_area 口径一致
        for r in [[5, 5, 5, 9], [5, 5, 9, 5], [9, 9, 5, 5], [5, 5, 6, 6]] {
            assert_eq!(rect_is_empty(r), rect_area(&r) == 0);
        }
        // i64 版必须与 i32 版**同口径**：一个判据两个入口，口径漂移既没有编译错误
        // 也没有崩溃，只会让 platform 侧与 core 侧对「退化」的理解悄悄分家。
        for r in [[5, 5, 5, 9], [5, 5, 9, 5], [9, 9, 5, 5], [5, 5, 6, 6]] {
            let w = [
                i64::from(r[0]),
                i64::from(r[1]),
                i64::from(r[2]),
                i64::from(r[3]),
            ];
            assert_eq!(rect_is_empty(r), rect_is_empty_i64(w), "rect {r:?}");
        }
        // i64 版存在的理由：贴到 i32 边界的极端虚拟桌面坐标，判据不许改变结论
        assert!(rect_is_empty_i64([
            i64::from(i32::MIN),
            0,
            i64::from(i32::MIN),
            0
        ]));
        assert!(!rect_is_empty_i64([
            i64::from(i32::MIN),
            0,
            i64::from(i32::MAX),
            1
        ]));
    }

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
    #[test]
    fn index_at_returns_closest_monitor_by_center_distance() {
        let rects = [([0, 0], [1000, 800]), ([1000, 0], [1200, 900])];
        assert_eq!(index_at(&rects, 0.0, 0.0), 0, "A 屏左上角");
        assert_eq!(index_at(&rects, 999.0, 799.0), 0, "A 屏右下角");
        assert_eq!(index_at(&rects, 1500.0, 500.0), 1, "B 屏内部");
        assert_eq!(index_at(&rects, -400.0, -300.0), 0);
        assert_eq!(index_at(&rects, 5000.0, 5000.0), 1);
        assert_eq!(index_at(&rects, f32::NAN, f32::NAN), 0);
        assert_eq!(index_at(&[], 100.0, 100.0), 0);
    }
}
