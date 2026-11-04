//! 自适应质量控制器（方案 12.4.3）。
//!
//! 判据只看「我们自己的工作」= prep + Pass A + Pass B + Commit，
//! **禁止**传入含采集等待与 Present 阻塞的 loop/render 时长（不变量 8）。

use std::time::Instant;

use crate::core::params::LiquidParams;

/// 自适应质量控制器（方案 12.4.3）。
///
/// `level 0` = 最高画质 = 本项目的默认视觉，**与未启用控制器时逐位一致**。
/// 只有实测持续丢 vblank 时才逐级降级，恢复后逐级升回：
///
/// 档位顺序按**实测收益**排，而不是按方案原文排：方案把"关焦散/关色散"放在
/// 限帧之前，但实测这两档只省少量 ALU、几乎不换帧率，会让控制器在真正有效的
/// 手段之前就白白牺牲视觉。实测收益：frost 半分辨率 +14% fps / −36.5 MB，
/// 限帧 60 直接把抖动变成平滑 60 fps，关焦散/关色散 ≈ 0。
///
/// | level | 动作 | 实测收益 |
/// |---|---|---|
/// | 1 | `frost` 保持 1/2 分辨率（省掉 Pass A 的整屏 tent 上采样） | +14% fps、−36.5 MB；模糊层差异 ≤3/255 |
/// | 2 | + 帧率上限 60（把"忽 8.33 忽 16.67 ms"的抖动变成恒定 16.67 ms） | 消除 judder |
/// | 3 | + 关闭焦散与 Beer-Lambert 吸收 | 少量 ALU（电池/过热时的余量） |
/// | 4 | + 关闭 RGB 色散（3 tap → 1 tap） | 省 2 次采样/像素 |
///
/// 判据用「丢 vblank 的比例」而不是 GPU timestamp：timestamp query 会打断驱动的
/// 命令批处理（实测把帧率打到 15 fps），用它做闭环会自我实现地降级。
/// 丢帧比例 = **我们自己那一帧的工作量**（渲染前准备 + Pass A + Pass B + Commit）
/// > 1.25 × 垂直消隐间隔 的**渲染帧**占比。
///
/// 为什么必须排除两段阻塞等待（它们都不是渲染成本）：
/// 1. 采集阶段等桌面新帧：桌面安静时驱动侧一等就是 8~32 ms；
/// 2. Present 等 vblank / DWM 背压：实测 6.6~25 ms（1~3 个消隐）。
///
/// 旧判据用整循环时长，把这两段都算成「丢帧」，结果单块拖拽也被判成
/// 「1 秒窗口 56 帧中丢 vblank 55%」、一路降到 level 2 —— 而我们自己的工作只有
/// 50 µs/帧（8.33 ms 预算的 1/160）。代价是白白把 frost 降成半分辨率（视觉改动），
/// fps 一点没变好（51.8 -> 51.0，瓶颈在触发节拍与 DWM，不在渲染）。
///
/// 已知取舍（诚实记录）：这个判据对「纯 GPU/DWM 背压」是盲的 —— 那种情况下
/// Present 阻塞变长而我们的提交时间不变。选它是因为误降档在本机是**每次拖拽
/// 必现**的实测缺陷，而纯 GPU 背压从未实测到过（最坏窗口里 A+B+C 仍只有 50 µs）。
/// 若将来真出现 GPU 背压场景，正确做法是先用 `DwmGetCompositionTimingInfo`
/// 拿到 DWM 实际合成节拍（混合刷新率下会变成 60 Hz），再按那个节拍判丢帧。
///
/// 防抖：连续 2 个 1 秒窗口超阈值才降档；连续 4 个窗口干净**且距上次降档 ≥20 s**
/// 才升档。后一条是为了避免周期振荡 —— 降到 1 档后变流畅、立刻升回 0 档又卡、
/// 再降，如此往复会让用户每隔二十几秒看到一次抖动。持续重负载下控制器会稳定
/// 停在 1 档，只有负载真的掉下来（删块、桌面变静）才升回。
///
/// `GLASS_ADAPTIVE=0` 可整体关闭（回到"永远 level 0"）。
pub(crate) struct QualityCtrl {
    enabled: bool,
    level: u8,
    vblank_us: u128,
    frames: u64,
    missed: u64,
    epoch: Instant,
    cooldown_until: Instant,
    last_downgrade: Instant,
    good_streak: u8,
    bad_streak: u8,
    min_interval: std::time::Duration,
    /// 上层 `EngineConfig::fps_cap` 给出的帧间隔下限。
    /// 与自适应档位的 `min_interval` 取**大者**生效：两者是「谁更保守听谁的」的关系。
    cap_interval: std::time::Duration,
    last_present: Instant,
    /// 升档探针的最小间隔。升上去 10 秒内又被打回 = 失败探针，间隔翻倍
    /// （上限 300 s），避免在"撑不住的档位"和"撑得住的档位"之间周期振荡。
    upgrade_gate: std::time::Duration,
    last_upgrade: Instant,
    /// 档位在本次 tick 里变过（供上层发事件 + 触发重画）。
    level_changed: bool,
}

/// 连击计数的上限（0.5.2 #6）。
///
/// 判据最多用到 4（连续 4 个干净窗口才升档、连续 2 个脏窗口才降档），16 已经是
/// 4 倍余量；再往上计数没有任何判据读得到，只是让计数器有机会溢出。
/// 显式夹紧（而不是只靠 `saturating_add` 停在 `u8::MAX`）还有一层意思：
/// 传入值若因别的原因超过上限，也会被拉回来，不会一路涨下去。
const STREAK_CAP: u8 = 16;

/// 一个评估窗口后的连击计数更新（纯函数，0.5.2 #6 的判据本体）。
///
/// 返回 `(good_streak, bad_streak)`。
///
/// 前科：`good_streak/bad_streak` 是 `u8`，旧实现用裸 `+ 1`。debug 构建开着
/// overflow 检查 ⇒ 连续 256 个同向窗口后 `attempt to add with overflow` **panic**，
/// 而 panic 发生在渲染线程的帧循环里 ⇒ 线程直接死掉，用户侧表现为
/// 「玻璃全灭 / 多操作几次分类框就消失」（约 256 s ≈ 4 分钟必现）。
/// release 构建不 panic 但会 wrap 回 0，同样把「连续 4 个干净窗口才升档」
/// 这条判据毁掉。两个方向都是**静默失效**，所以判据必须写成纯函数 + 顶到
/// 上限再跑一轮的单测（`streak_overflow_does_not_panic`）。
///
/// 现在：`saturating_add` 并夹到 [`STREAK_CAP`] —— 到顶就停在顶上，不增也不回绕。
#[must_use]
pub(crate) fn bump_streaks(
    miss_rate: f64,
    good: u8,
    bad: u8,
    bad_th: f64,
    good_th: f64,
) -> (u8, u8) {
    if miss_rate > bad_th {
        (0, bad.saturating_add(1).min(STREAK_CAP))
    } else if miss_rate < good_th {
        (good.saturating_add(1).min(STREAK_CAP), 0)
    } else {
        // 落在两个阈值之间：两边都清零（与旧实现一致 —— 只有「明确干净」
        // 或「明确脏」才累计，模棱两可的窗口不投票）。
        (0, 0)
    }
}

/// 「很久以前」的时间点。
///
/// `Instant - Duration` 在越界时会 **panic**（spec 明令禁止裸 `Instant - Duration`），
/// 因此一律走 `checked_sub` 并退化到 `now()`。
fn far_past() -> Instant {
    Instant::now()
        .checked_sub(std::time::Duration::from_secs(3600))
        .unwrap_or_else(Instant::now)
}

impl QualityCtrl {
    pub(crate) fn new(hz: f32) -> Self {
        Self {
            // 是否启用由 EngineConfig / GLASS_ADAPTIVE 决定（diag::env 统一解析后调 disable），
            // 本层不再自己读环境变量。
            enabled: true,
            level: 0,
            vblank_us: (1_000_000.0 / hz.max(1.0)) as u128,
            frames: 0,
            missed: 0,
            epoch: Instant::now(),
            cooldown_until: Instant::now(),
            last_downgrade: far_past(),
            good_streak: 0,
            bad_streak: 0,
            min_interval: std::time::Duration::from_secs(0),
            cap_interval: std::time::Duration::from_secs(0),
            last_present: Instant::now(),
            upgrade_gate: std::time::Duration::from_secs(20),
            last_upgrade: far_past(),
            level_changed: false,
        }
    }

    /// 关闭**自适应降档**（回到「永远 level 0」，与未启用时逐位一致）。
    ///
    /// 只清自适应那一份 `min_interval`，**保留** `cap_interval`：
    /// `adaptive_quality` 与 `fps_cap` 是两个正交旋钮，而 `GLASS_ADAPTIVE=0`
    /// 是每一个 verify_* 取证脚本都会设的 —— 若连 cap 一起清，
    /// 「fps_cap + 取证」这个组合的帧率上限会静默失效（独立审查 P1-5）。
    pub(crate) fn disable(&mut self) {
        self.enabled = false;
        self.level = 0;
        self.min_interval = std::time::Duration::from_secs(0);
    }

    /// 设置帧率上限（`EngineConfig::fps_cap`）。`None` = 跟随显示器刷新率。
    pub(crate) fn set_fps_cap(&mut self, cap: Option<u32>) {
        self.cap_interval = match cap {
            Some(c) if c > 0 => std::time::Duration::from_secs_f64(1.0 / f64::from(c)),
            _ => std::time::Duration::from_secs(0),
        };
    }

    /// 实际生效的帧间隔下限 = max(自适应档位, 上层 fps_cap)。
    #[must_use]
    fn eff_min_interval(&self) -> std::time::Duration {
        self.min_interval.max(self.cap_interval)
    }

    /// 是否启用。
    #[must_use]
    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    /// 当前档位（0 = 不降档）。
    #[must_use]
    pub(crate) fn level(&self) -> u8 {
        self.level
    }

    /// 取走「档位刚变过」标记（上层据此发 `QualityLevelChanged` 事件并触发重画）。
    pub(crate) fn take_level_changed(&mut self) -> bool {
        std::mem::take(&mut self.level_changed)
    }

    #[cfg(test)]
    pub(crate) fn set_level_for_test(&mut self, level: u8) {
        self.level = level;
        self.min_interval = if level >= 2 {
            std::time::Duration::from_millis(1000 / 60)
        } else {
            std::time::Duration::from_secs(0)
        };
    }

    /// level 4 的帧率上限：本帧是否允许提交
    pub(crate) fn allow_present(&self, now: Instant) -> bool {
        let iv = self.eff_min_interval();
        iv.is_zero() || now.duration_since(self.last_present) >= iv
    }

    /// 丢帧判定：渲染段耗时超过 1.25 × 目标帧间隔。抽成纯函数供单测。
    pub(crate) fn is_miss(loop_us: u128, target_us: u128) -> bool {
        loop_us > target_us + target_us / 4
    }

    pub(crate) fn note_present(&mut self, now: Instant) {
        self.last_present = now;
    }

    /// 推进一个判定窗口。返回本次是否发生了档位变化。
    pub(crate) fn tick(&mut self, loop_us: u128, rendered: bool) -> bool {
        if !self.enabled {
            return false;
        }
        if rendered {
            self.frames += 1;
            let target = self.eff_min_interval().as_micros().max(self.vblank_us);
            if Self::is_miss(loop_us, target) {
                self.missed += 1;
            }
        }
        if self.epoch.elapsed() < std::time::Duration::from_secs(1) {
            return false;
        }
        let frames = self.frames;
        // 渲染帧很少 = 基本空闲 = 没有负载，视为干净窗口。否则降档之后
        // 一旦桌面变静就永远凑不满判定窗口，档位升不回来。
        let miss_rate = if frames >= 10 {
            self.missed as f64 / frames as f64
        } else {
            0.0
        };
        self.frames = 0;
        self.missed = 0;
        self.epoch = Instant::now();
        if Instant::now() < self.cooldown_until {
            return false;
        }
        // 升到更低档位后稳住了 30 s：说明这次探针是成功的，恢复默认探针间隔
        if self.level > 0
            && Instant::now().duration_since(self.last_upgrade)
                >= std::time::Duration::from_secs(30)
        {
            self.upgrade_gate = std::time::Duration::from_secs(20);
        }
        // 电池供电时更保守：阈值收紧
        let (bad_th, good_th) = if crate::platform::display::on_battery() {
            (0.15, 0.03)
        } else {
            (0.25, 0.05)
        };
        // 0.5.2 #6：连击计数走纯函数 + saturating（裸 `+1` 在 debug 构建里
        // 256 个同向窗口后 panic 杀渲染线程）。判据本体见 [`bump_streaks`]。
        let (good, bad) = bump_streaks(
            miss_rate,
            self.good_streak,
            self.bad_streak,
            bad_th,
            good_th,
        );
        self.good_streak = good;
        self.bad_streak = bad;
        if self.bad_streak >= 2 && frames >= 30 && self.level < 4 {
            self.set_level(self.level + 1, miss_rate, frames);
        } else if self.good_streak >= 4
            && self.level > 0
            && Instant::now().duration_since(self.last_downgrade) >= self.upgrade_gate
        {
            self.set_level(self.level - 1, miss_rate, frames);
        }
        self.level_changed
    }

    fn set_level(&mut self, level: u8, miss_rate: f64, frames: u64) {
        self.level_changed = true;
        // 切换档位会重建金字塔，产生一次停顿；清空统计窗口，别让这次停顿
        // 被算成"还在丢帧"从而触发下一级降级。
        self.frames = 0;
        self.missed = 0;
        self.epoch = Instant::now();
        let up = level < self.level;
        if !up
            && Instant::now().duration_since(self.last_upgrade) < std::time::Duration::from_secs(10)
        {
            // 刚升上去就被打回：这次探针失败，下次探针间隔翻倍
            self.upgrade_gate = (self.upgrade_gate * 2).min(std::time::Duration::from_secs(300));
        }
        if up {
            self.last_upgrade = Instant::now();
        }
        self.level = level;
        self.bad_streak = 0;
        self.good_streak = 0;
        self.cooldown_until = Instant::now() + std::time::Duration::from_secs(3);
        if !up {
            self.last_downgrade = Instant::now();
        }
        self.min_interval = if level >= 2 {
            std::time::Duration::from_millis(1000 / 60)
        } else {
            std::time::Duration::from_secs(0)
        };
        log::info!(
            target: "adaptive",
            "质量档位{} -> {level}（1 秒窗口 {frames} 帧中丢 vblank {:.0}%）：{}",
            if up { "回升" } else { "降级" },
            miss_rate * 100.0,
            match level {
                0 => "全部效果开启，与默认视觉逐位一致",
                1 => "frost 降为 1/2 分辨率（模糊层差异 ≤3/255）",
                2 => "+ 帧率上限 60（抖动 -> 恒定 60 fps）",
                3 => "+ 关闭焦散与 Beer-Lambert 吸收",
                _ => "+ 关闭 RGB 色散",
            }
        );
    }

    #[must_use]
    pub(crate) fn frost_half(&self) -> bool {
        self.enabled && self.level >= 1
    }

    /// 把 level 3/4 的参数降级叠加到调用方参数之上（level 0~2 时逐字段原样）。
    ///
    /// **逐块**调用：每个块的有效参数来自它自己的样式，降档也必须逐块施加。
    /// level < 3 时是恒等变换 —— 这是「level 0 与未启用时逐位一致」的保证。
    pub(crate) fn apply(&self, p: &mut LiquidParams) {
        if !self.enabled || self.level < 3 {
            return;
        }
        p.caustic_gain = 0.0;
        p.absorption = 0.0;
        if self.level >= 4 {
            p.dispersion = 0.0;
        }
    }
}

/// `GLASS_ADAPTIVE=0`（每个 verify_* 取证脚本都设）不能顺手把上层的
/// `fps_cap` 一起清掉 —— 两个是正交旋钮（独立审查 P1-5 的回归测试）。
#[test]
fn disable_keeps_the_fps_cap() {
    let mut q = QualityCtrl::new(120.0);
    q.set_fps_cap(Some(30));
    q.disable();
    let now = Instant::now();
    assert!(
        !q.allow_present(now),
        "30 fps 上限在 disable 之后必须仍然生效"
    );
    assert!(
        q.allow_present(now + std::time::Duration::from_millis(34)),
        "超过 1/30s 之后必须放行"
    );
    assert!(!q.enabled());
    assert_eq!(q.level(), 0);
}

/// `set_fps_cap` 与自适应档位取**大者**：cap 比档位更保守时 cap 赢。
#[test]
fn fps_cap_and_adaptive_level_take_the_stricter_one() {
    let mut q = QualityCtrl::new(120.0);
    q.set_fps_cap(Some(30)); // 33.3 ms
                             // level 2 的 min_interval = 16.67 ms，比 cap 宽松 -> 生效的必须是 cap
    q.set_level_for_test(2);
    let now = Instant::now();
    assert!(!q.allow_present(now + std::time::Duration::from_millis(20)));
    assert!(q.allow_present(now + std::time::Duration::from_millis(34)));
}

#[cfg(test)]
mod tests {
    use super::{bump_streaks, far_past, QualityCtrl, STREAK_CAP};

    /// 0.5.2 #6 主判据：计数器顶到上限后再跑一轮评估，不许 panic、不许回绕。
    ///
    /// 判别力：把实现改回裸 `self.good_streak += 1`，本测试在 **debug** 构建里
    /// 立刻 `attempt to add with overflow` panic（这正是宿主线上「约 256 s 必崩」
    /// 的那条路径）；改回 `wrapping_add` 则断言 2 的「停在顶上」会红。
    #[test]
    fn streak_overflow_does_not_panic() {
        // 1) 纯函数：顶到上限再 +1，值必须夹紧而不是回绕。
        let (g, b) = bump_streaks(0.0, u8::MAX, 0, 0.25, 0.05);
        assert_eq!((g, b), (STREAK_CAP, 0), "干净窗口到顶后必须夹在上限");
        let (g, b) = bump_streaks(1.0, 0, u8::MAX, 0.25, 0.05);
        assert_eq!((g, b), (0, STREAK_CAP), "脏窗口到顶后必须夹在上限");
        // 上限之上的传入值也要被拉回来（不只是「不再涨」）。
        assert_eq!(bump_streaks(0.0, 200, 0, 0.25, 0.05).0, STREAK_CAP);
        // NaN 不进任何分支（两个比较都是 false）⇒ 归到「模棱两可」清零，绝不 panic。
        assert_eq!(bump_streaks(f64::NAN, 7, 7, 0.25, 0.05), (0, 0));

        // 2) 端到端：把控制器顶到上限，再连跑 300 轮评估（旧实现第 1 轮就炸）。
        let mut q = QualityCtrl::new(60.0);
        q.level = 0; // level 0 时「连续 4 个干净窗口」不会触发升档，计数得以留在顶上
        q.good_streak = u8::MAX;
        for _ in 0..300 {
            q.epoch = far_past();
            q.cooldown_until = far_past();
            q.frames = 30;
            q.missed = 0;
            assert!(!q.tick(1_000, true), "level 0 且干净：不该改档位");
        }
        assert_eq!(
            q.good_streak, STREAK_CAP,
            "300 轮干净评估后必须夹在上限（既没回绕也没继续涨）"
        );
        assert_eq!(q.level, 0);

        // 3) 脏方向同样不许炸（level 4 = 最低档，`level < 4` 为假 ⇒ 不再降档）。
        q.level = 4;
        q.bad_streak = u8::MAX;
        for _ in 0..300 {
            q.epoch = far_past();
            q.cooldown_until = far_past();
            q.frames = 30;
            q.missed = 30;
            q.tick(10_000, true);
        }
        assert_eq!(q.bad_streak, STREAK_CAP, "300 轮脏评估后必须夹在上限");
    }

    #[test]
    fn miss_threshold_is_125_percent_of_target() {
        let t = 8333u128; // 120 Hz
        assert!(!QualityCtrl::is_miss(t, t));
        assert!(!QualityCtrl::is_miss(t + t / 4, t)); // 恰好 1.25x 不算丢
        assert!(QualityCtrl::is_miss(t + t / 4 + 1, t));
        // 采集等待不得被计入：判据收到的必须是「我们自己的工作」
        assert!(!QualityCtrl::is_miss(90, t)); // 90 µs 的真实工作量绝不是丢帧
    }
    #[test]
    fn miss_threshold_respects_fps_cap_level() {
        let t = 16667u128; // level>=2 的 60 fps 上限
        assert!(!QualityCtrl::is_miss(16700, t));
        assert!(QualityCtrl::is_miss(21000, t));
    }
}
