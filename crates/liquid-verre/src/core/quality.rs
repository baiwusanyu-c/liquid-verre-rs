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
        if miss_rate > bad_th {
            self.bad_streak += 1;
            self.good_streak = 0;
        } else if miss_rate < good_th {
            self.good_streak += 1;
            self.bad_streak = 0;
        } else {
            self.good_streak = 0;
            self.bad_streak = 0;
        }
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
    use super::QualityCtrl;

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
