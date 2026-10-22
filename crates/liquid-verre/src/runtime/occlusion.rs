//! 逐块遮挡检测的**编排层**：8 Hz 扫一遍全桌面 z 序，把结论写回 `Block::occluded`。
//!
//! 这里是 `Block::occluded` / `Block::occl_streak` 的**唯一写点**（spec 的「单一来源」硬规矩）。
//! 分层职责一行都不越界：
//!
//! - 矩形并集覆盖判定（纯函数、可单测）→ `core::geometry::rect_fully_covered`
//! - 迟滞状态机（纯函数、可单测）→ `core::block::occlusion_step`
//! - Win32 z 序枚举（`unsafe` 集中处）→ `platform::occlusion::opaque_rects_above`
//! - **本模块**：只决定「什么时候扫、扫哪一块、结论写回谁、什么时候打日志」，
//!   一行几何运算与一行 Win32 调用都不自己写。
//!
//! ## 为什么是 8 Hz 轮询而不是事件驱动
//!
//! `EnumWindows` + 逐窗口 `GetWindowRect` / `GetWindowRgn` / `DwmGetWindowAttribute` 的成本
//! 与桌面上的窗口数成正比，绝不能进每帧热路径（R-B1-3）。125 ms 一轮 ⇒ 置位最坏 250 ms
//! （迟滞两轮，`OCCLUSION_CONFIRM_POLLS = 2`）、解除最坏 125 ms。
//! `SetWinEventHook` 能做到亚帧恢复，但代价是「任意窗口移动都唤醒我们」，
//! 在繁忙桌面上比 8 Hz 轮询贵得多 —— 本任务明确不做（PRD Non-Goals）。
//!
//! ## 已知取舍（别当 bug 修）
//!
//! 遮挡移开后的 ≤125 ms 里玻璃内是**陈旧桌面**：停产期间一帧都没采集。恢复那一帧会走
//! `just_activated` → 32 ms 阻塞采集 + `compositor.invalidate()`（整屏 Pass A + 全量重画，
//! 见 `runtime/mod.rs`），因此不影响正确性，只影响那一小段观感。
//!
//! 判据用 `draw_rect()`（几何矩形 ⊕ 阴影外扩）而**不外扩缩放命中带** `grip`：命中带只是
//! 「能从块外多远抓住边缘」的**输入**范围，不产生任何像素 —— 缩放反馈的 border 是画在块自身的
//! 圆角矩形上的（独立叠加 pass，见 BASELINE 不变量 5）。所以 `draw_rect` 已经是该块可见像素的
//! 完整硬边界，再外扩只会白丢收益。命中测试与点击穿透（`inside`）也**不读** `occluded`：
//! 块被盖住时鼠标本来也落在上面那个窗口上，改穿透语义只会引入新的不一致。
//!
//! ## 日志（`logging-guidelines.md`：前缀即接口）
//!
//! 全部走**级别名**，不新增 `RAW_TARGETS`；全部在 8 Hz 的 `update()` 里、且只在**翻转沿**触发
//! （R-C3：热路径禁止格式化日志）。逐块翻转 = `debug`（可能一轮好几条，排障才需要）；
//! 每屏「全部块被遮挡 ⇒ 采集停产 / 恢复」= `info`，与既有的
//! 「检测到独占全屏应用，暂时隐藏玻璃」同一量级。

use std::time::{Duration, Instant};

use crate::core::block::occlusion_step;
use crate::core::geometry::rect_fully_covered;
use crate::platform;
use crate::runtime::app::App;

/// 遮挡轮询间隔：125 ms = 8 Hz。
///
/// 与「独占全屏检测」的 250 ms（4 Hz，`App::fs_check_at`）同构但更密：全屏应用的进出是
/// 秒级的人类事件，而遮挡窗口可能一次 `Alt+Tab` 就换掉，太稀会把「玻璃里是旧桌面」
/// 的时间窗拉长到肉眼可见。再密就没意义了 —— 结论本身要连续两轮才置位。
const OCCLUSION_POLL: Duration = Duration::from_millis(125);

/// 把**画布局部**矩形平移到**虚拟屏绝对**坐标（半开区间 `[x0,y0,x1,y1]`）。
///
/// widen 到 `i64` 再夹回 `i32`：多屏虚拟桌面的原点与窗口坐标都是不可信输入，
/// debug 构建下裸 `i32` 加法溢出即 panic（spec 禁止「裸外部整数加减」）。
/// 夹住而不是丢弃：判据方向必须是「多渲染」，退化的矩形在 `rect_fully_covered`
/// 与 `opaque_rects_above` 里都会被当成「零面积 ⇒ 不遮挡」处理。
#[must_use]
fn to_abs(r: [i32; 4], ox: i32, oy: i32) -> [i32; 4] {
    let shift = |v: i32, o: i32| {
        (i64::from(v) + i64::from(o)).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    };
    [
        shift(r[0], ox),
        shift(r[1], oy),
        shift(r[2], ox),
        shift(r[3], oy),
    ]
}

/// 第 `mi` 块屏在 [`App::occl_paused_mask`] 里的状态位；屏下标 >= 32 时返回 `None`。
///
/// `None` 的语义是**「这一屏的日志翻转沿记不住」**，调用方必须因此**一条日志都不打** ——
/// 不是「少打一点」。修之前的写法是「记不住位就照常打」，后果是 `was` 恒为 `false`：
/// 只要该屏全部块被遮挡，就会**每 125 ms 重打一条**「桌面采集停产」，永远停不下来，
/// 正好是 R-C3 与 `logging-guidelines.md` 明令禁止的周期性日志（前科：warn 刷屏把
/// `[metrics]` 行冲走，`perf_probe.py` / `gate_perf.py` 直接读不到数据 ⇒ 阀门瞎掉）。
///
/// 抽成纯函数是为了可测：>32 屏的桌面造不出来，但这条边界必须有断言钉住。
#[must_use]
fn paused_bit(mi: usize) -> Option<u32> {
    u32::try_from(mi).ok().and_then(|i| 1u32.checked_shl(i))
}

/// 8 Hz 轮询：更新每块的 `occluded`，并在翻转沿打日志。
///
/// 调用点在 `runtime/mod.rs` 的 prep 段 —— `collect_monitor_blocks` 的 per-monitor 循环
/// **之后**（要读它刚投影出来的 `mon.blocks`）、渲染门控**之前**。因此本轮结论被
/// `mouse_relevant` / 采集需求扫描读到时会滞后 ≤1 圈（≤8 ms）+ 轮询间隔；
/// 对 8 Hz 的决策毫无意义，换来的是 `Block::occluded` 是唯一真值、不存在两处状态漂移。
pub(crate) fn update(app: &mut App) {
    // 关掉时连时间戳都不必维护：AC7 要求「关开关 ⇒ 与改动前逐位一致」，
    // 而维护 `occl_check_at` 本身就是一种（虽然极小的）可观察差异。
    // 这个 bool 是构造期算好的（`cfg.occlusion_culling && !env.no_occlusion`），
    // 热路径绝不 `env::var`。
    if !app.occlusion_enabled {
        return;
    }
    // `elapsed()` 内部是 saturating 减法，不会像裸 `Instant - Duration` 那样越界 panic（R-D5）。
    if app.occl_check_at.elapsed() < OCCLUSION_POLL {
        return;
    }
    app.occl_check_at = Instant::now();

    // 被拖拽的那一块**强制**判「未遮挡」：拖拽中的块必然在鼠标下可见，
    // 若被判遮挡就会停采 → 玻璃里冻在旧桌面 → 拖拽看起来卡顿。
    // 这是最难查的间歇问题（只在「拖到某个恰好被覆盖的位置」时复现），直接在源头排除。
    let dragged = app.drag.as_ref().map(|d| d.id());

    // 解构而不是逐字段借用：`monitors`（不可变）与 `manager`（可变）是**不同字段**，
    // 分开借出后才能在同一个循环里「读本屏投影块 + 写回权威块状态」。
    let App {
        monitors,
        manager,
        occluders,
        occl_paused_mask,
        ..
    } = app;

    for (mi, mon) in monitors.iter().enumerate() {
        // 每屏一个状态位，只服务「停采 / 恢复」的日志翻转沿。
        // 用 u32 位掩码而不是 `Vec<bool>`：不必跟着 `rebuild_monitors` resize，
        // 也永远不会在轮询里分配。超过 32 块屏时**完全放弃**这一屏的翻转沿日志
        // （`paused_bit` 给出 `None`，见其文档：记不住状态就不许打）——
        // 遮挡判定本身仍逐块正确，不值得为它上 Vec。
        let bit = paused_bit(mi);

        // 本屏没有块：既没有可被覆盖的目标，也不算「因遮挡而停采」。
        // 静默清位（不打日志），否则「屏变空 → 又有块且被遮住」这条路径会漏掉翻转沿。
        if mon.blocks.is_empty() {
            if let Some(b) = bit {
                *occl_paused_mask &= !b;
            }
            continue;
        }

        // 原点直接取 `info.origin`（i32）而不是 `canvas_origin()`（f32）：后者是
        // `origin as f32`，坐标绝对值超过 2^24 就丢精度，而遮挡判据差 1 px
        // 就可能让「完整覆盖」判不出来（半开区间的意义见 `rect_fully_covered` 文档）。
        let (ox, oy) = (mon.info.origin[0], mon.info.origin[1]);

        // bounds = 本屏所有块**绘制矩形**的并集。
        // 为什么用 `draw_rect()`（含阴影外扩）而不是几何矩形：Pass B 是 per-block
        // viewport + scissor，绘制矩形才是该块可见像素的**硬边界**；用它判「完整覆盖」
        // ⇒ 该块贡献 0 个可见像素 ⇒ 它的一切触发源都可证明是浪费（R-B1-1）。
        let mut bx0 = i64::MAX;
        let mut by0 = i64::MAX;
        let mut bx1 = i64::MIN;
        let mut by1 = i64::MIN;
        for blk in mon.blocks.iter() {
            let r = blk.draw_rect();
            bx0 = bx0.min(i64::from(r[0]) + i64::from(ox));
            by0 = by0.min(i64::from(r[1]) + i64::from(oy));
            bx1 = bx1.max(i64::from(r[2]) + i64::from(ox));
            by1 = by1.max(i64::from(r[3]) + i64::from(oy));
        }
        let fit = |v: i64| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        let bounds = [fit(bx0), fit(by0), fit(bx1), fit(by1)];

        // ⚠ `opaque_rects_above` **只追加、不清空** `out`（多屏共用同一个跨帧缓冲区，
        // 热路径零分配），因此每屏调用前必须 clear，否则上一屏的遮挡者会串到这一屏。
        // bounds 退化（零面积 / 反序）时它自己会直接返回 0 且不进 EnumWindows。
        occluders.clear();
        let found = platform::occlusion::opaque_rects_above(mon.hwnd, bounds, occluders);

        let mut all_occluded = true;
        for blk in mon.blocks.iter() {
            let abs = to_abs(blk.draw_rect(), ox, oy);
            // found == 0 ⇒ covers 为空 ⇒ `rect_fully_covered` 立刻返回 false
            // （解除只需 1 轮，错判方向永远是「多渲染」）。
            let mut hit = rect_fully_covered(abs, occluders.as_slice());
            if dragged == Some(blk.id) {
                hit = false;
            }
            // `mon.blocks` 与 `manager` 在同一圈 prep 里投影，中间不消费命令，
            // 因此这里取不到只可能是块已被销毁：跳过即可（下一圈投影就没有它了）。
            let Some(b) = manager.get_mut(blk.id) else {
                continue;
            };
            let (occ, streak) = occlusion_step(b.occl_streak, hit);
            b.occl_streak = streak;
            if occ != b.occluded {
                b.occluded = occ;
                log::debug!(
                    "块 {} 遮挡判定翻转 → {}",
                    b.id,
                    if occ {
                        "被完整覆盖"
                    } else {
                        "重新可见"
                    }
                );
            }
            all_occluded = all_occluded && occ;
        }

        // 每屏「因全部块被遮挡而停产 / 恢复」的翻转沿。措辞刻意与 A（`hidden_fs`）对齐：
        // 两者症状相同（`[metrics]` 的 dda_acq 归零），排障时必须一眼分清是
        // 「整屏被全屏应用压住」还是「块被普通窗口盖住」。
        // 注意这是**日志口径**而非采集口径：块数超过 16 屏上限时主循环的 `overflow`
        // 兜底会让采集照常（见 `runtime/mod.rs` 的 `wants_bd`），日志不假装知道那件事。
        let was = bit.is_some_and(|b| *occl_paused_mask & b != 0);
        if all_occluded != was {
            // 日志**必须**与状态位绑在同一个 `if let` 里：`bit == None`（>32 屏）时
            // 记不住状态 ⇒ 一条都不许打，否则就退化成每 125 ms 一条的周期性日志
            // （见 `paused_bit` 的文档）。改这里之前先读那段。
            if let Some(b) = bit {
                if all_occluded {
                    *occl_paused_mask |= b;
                    log::info!(
                        "{} 全部玻璃块被不透明窗口完整遮挡（遮挡者 {found} 个），桌面采集停产",
                        mon.info.name
                    );
                } else {
                    *occl_paused_mask &= !b;
                    log::info!("{} 遮挡已移开，恢复桌面采集", mon.info.name);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{paused_bit, to_abs, OCCLUSION_POLL};

    #[test]
    fn to_abs_shifts_both_corners() {
        assert_eq!(to_abs([1, 2, 3, 4], 10, 20), [11, 22, 13, 24]);
        // 副屏原点是负数（左屏当副屏的排布），平移必须照样正确
        assert_eq!(to_abs([-5, -5, 5, 5], -100, 100), [-105, 95, -95, 105]);
        assert_eq!(to_abs([0, 0, 0, 0], 0, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn to_abs_saturates_instead_of_panicking() {
        // 多屏虚拟桌面能给到 ±2^31 量级的坐标：debug 构建下裸 i32 加法在这里就 panic。
        // 夹住的方向是安全的 —— 退化矩形在 `rect_fully_covered` 里判「零面积 ⇒ 已覆盖」，
        // 而 bounds 退化在 `opaque_rects_above` 里判「不扫 ⇒ 没有遮挡者」，两边都偏保守。
        assert_eq!(
            to_abs([i32::MAX, i32::MIN, 0, 0], i32::MAX, i32::MIN),
            [i32::MAX, i32::MIN, i32::MAX, i32::MIN]
        );
    }

    #[test]
    fn paused_bit_is_unique_below_32_and_gives_up_above() {
        assert_eq!(paused_bit(0), Some(1));
        assert_eq!(paused_bit(1), Some(2));
        assert_eq!(paused_bit(31), Some(1 << 31));
        // >= 32：`checked_shl` 给 None ⇒ 调用方必须**完全不打**这一屏的翻转沿日志。
        // 若还照打，`was` 恒为 false，就是每 125 ms 一条的周期性日志（R-C3 禁止）。
        assert_eq!(paused_bit(32), None);
        assert_eq!(paused_bit(33), None);
        assert_eq!(paused_bit(usize::MAX), None);
        // 位与位之间不许重叠：重叠会让两块屏的日志状态互相踩，
        // 于是「A 屏恢复」把「B 屏停产」的位清掉，翻转沿日志要么漏要么重。
        for mi in 0..32usize {
            for mj in (mi + 1)..32usize {
                let (a, b) = (paused_bit(mi), paused_bit(mj));
                match (a, b) {
                    (Some(x), Some(y)) => assert_eq!(x & y, 0, "屏 {mi} 与 {mj} 的位重叠"),
                    _ => panic!("0..32 之内必须都拿到位"),
                }
            }
        }
    }

    #[test]
    fn poll_interval_matches_the_documented_latency_budget() {
        // 125 ms = 8 Hz：PRD 承诺「置位 ≤250 ms（连续 2 轮）、解除 ≤125 ms」。
        // 这条单测钉住常量与承诺的关系 —— 谁把间隔调大，就必须同时改文档与验收口径。
        assert_eq!(OCCLUSION_POLL.as_millis(), 125);
        assert!(
            OCCLUSION_POLL.as_millis() * 2 <= 250,
            "置位延迟不得超过 250 ms"
        );
        assert!(
            OCCLUSION_POLL.as_millis() >= 100,
            "再密就进不了「非热路径」的成本模型"
        );
    }
}
