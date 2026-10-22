//! `GLASS_*` 诊断开关的**唯一**解析点。
//!
//! 范围：这里只集中**主循环**需要的那一批开关。另有几个开关天生属于某个子系统、
//! 只在那一处读取（`GLASS_COPY_MODE` / `GLASS_SIM_DIRTY` / `GLASS_SKIP_COPY` 在
//! `capture/dda.rs`，`GLASS_ENV_EMA` / `GLASS_FROST_HALF` / `GLASS_PASS_A_HZ` 在
//! `gfx/pyramid.rs`，`GLASS_SHOT_EVERY_MS` / `GLASS_SKIP_*` / `GLASS_FORCE_FULL` /
//! `GLASS_VERIFY_VALIDITY` / `GLASS_INTERNAL_SHOT_AT_MS` 在 `gfx/compositor.rs`，
//! `GLASS_SYNTH_CAP` 在 `capture/mod.rs`，`GLASS_FRAME_LATENCY` 在 `gfx/device.rs`）。
//! 它们都只在**构造期**读一次，不进热路径，因此留在原地比强行搬过来更清晰。
//!
//! 为什么集中：重构前 `std::env::var` 散落在 `main()` 与 `capture/dda.rs` 里，
//! 其中一处曾经被写进每帧循环 —— 每帧加一次进程环境锁并分配一个 `String`
//! （见 `.trellis/spec/backend/quality-guidelines.md` 的「禁止模式」）。
//! 现在统一在引擎启动时解析一次，热路径只读结构体字段。
//!
//! 其中 `GLASS_ADAPTIVE=0` 与 `GLASS_OCCLUSION=0` 是**同一类**：把某个产品正向开关
//! （`EngineConfig::adaptive_quality` / `EngineConfig::occlusion_culling`）临时关掉的取证后门，
//! 供 `tools/` 做 A/B 归因。两者都只在构造期读一次，与配置字段合成一个 bool 存进运行期状态
//! （`QualityCtrl::disable()` / `App::occlusion_enabled`），热路径绝不 `env::var`。
//!
//! `GLASS_FS_HIDE=0` 也属于这一类，但它关掉的是「独占全屏应用检测」（方案 R-16）。
//! 该检测的形状判据是「前台窗口 = `WS_POPUP` 且无 caption/thickframe 且铺满整块屏」，
//! 而**无边框最大化的普通应用**（Electron / Tauri 那一类）完全符合这个形状 ——
//! 玻璃确实被它盖住了，隐藏 + 停产在**产品语义**上是对的；但对 `tools/` 的取证脚本是灾难：
//! 画布一停产，连拍拿到 0 帧、拖拽类脚本整批 FAIL（实测：前台恰好是这类应用时
//! `repro_tremble far/many` 与 `verify_adaptive` 直接废掉，且与 `GLASS_OCCLUSION` 无关）。
//! 所以像素 / 性能取证一律显式设 `GLASS_FS_HIDE=0`，让闸门对「用户前台开着哪个应用」
//! 保持 hermetic —— 与 `GLASS_ADAPTIVE=0`（不许降档中途改视觉）同一个理由。
//!
//! 环境变量名保持 `GLASS_*` 不变：`tools/` 下 8 个回归脚本与 `gate_perf.py`
//! 全部按这些名字取证，改名等于打断质量阀门。上层应用应当优先用
//! [`crate::api::DiagnosticsConfig`]，环境变量只是给脚本 / 命令行用的后门。

use std::time::Duration;

/// 解析后的诊断开关集合。
#[derive(Debug, Clone, Default)]
pub(crate) struct EnvOverrides {
    /// `GLASS_METRICS=1`：每秒打印一行 `[metrics]`。
    pub metrics: bool,
    /// `GLASS_DEFAULT_BLOCK=1`：启动即建一块居中玻璃。
    ///
    /// 库默认**不**自建窗口（那是上层应用的职责）；`tools/` 下的取证脚本需要
    /// 一块确定性的默认玻璃来做逐位比对，因此由脚本显式打开。
    pub default_block: bool,
    /// `GLASS_MANY=1`：启动即铺 20 块（压力测试）。
    pub many: bool,
    /// `GLASS_SPREAD=1`：每屏正中各放一块（多屏验收）。
    pub spread: bool,
    /// `GLASS_FAR_GRID=1`：4 块互不重叠的确定性布局（鼠标隔离硬证明）。
    pub far_grid: bool,
    /// `GLASS_EXTREME_SPEC=1`：最大高光强度 + 最小锐度（圆角断裂回归）。
    pub extreme_spec: bool,
    /// `GLASS_SIM_BUSY=<ms>`：拖拽帧注入确定性忙等（验证自适应控制器闭环）。
    pub sim_busy: Duration,
    /// `GLASS_SIM_BACKDROP=<W>x<H>@<HZ>`：确定性模拟桌面脏区。
    pub sim_backdrop: Option<(i32, i32, f64)>,
    /// `GLASS_MONITORS=<n>`：只用前 n 块屏。
    pub monitor_limit: Option<usize>,
    /// `GLASS_VERIFY_PASS_A=1`：拍一张脏矩形受限帧后再拍一张全量对照帧。
    pub verify_pass_a: bool,
    /// `GLASS_INTERNAL_SHOT=<path>`：取证截图输出路径（按屏加 `.m<i>.bmp`）。
    pub internal_shot: Option<String>,
    /// `GLASS_CURSOR_AT=x,y`：固定初始光标位置（回归测试）。
    pub cursor_at: Option<[f32; 2]>,
    /// `GLASS_ADAPTIVE=0`：关闭自适应质量控制器。
    pub no_adaptive: bool,
    /// `GLASS_OCCLUSION=0`：关闭逐块遮挡剔除（B1 的取证后门，与 `no_adaptive` 完全同构）。
    ///
    /// 只在 `runtime::run()` 里读一次并与 `EngineConfig::occlusion_culling` 合成
    /// `App::occlusion_enabled`；8 Hz 轮询与两个消费点只读那个 bool。
    /// 关掉之后 `Block::occluded` 恒为 `false`，两处消费点短路 ⇒ 行为与改动前逐位一致（AC7）。
    pub no_occlusion: bool,
    /// `GLASS_FS_HIDE=0`：关闭「独占全屏应用检测」（方案 R-16）的隐藏 + 停产。
    ///
    /// 这是产品开关 [`crate::api::EngineConfig::fullscreen_hide`] 的**取证后门**，
    /// 与 `no_adaptive` / `no_occlusion` 完全同构：`runtime::run()` 构造期读一次并合成
    /// `fs_hide_enabled = cfg.fullscreen_hide && !env.no_fs_hide`，4 Hz 检测块只读那个 bool。
    ///
    /// 取证脚本必须设它，否则闸门不 hermetic：该检测的形状判据（**前台**窗口 = `WS_POPUP`
    /// 且无 caption/thickframe 且铺满整块屏）与「无边框最大化的普通应用」重合，
    /// 一旦误触发就是**整屏停产** ⇒ `tools/` 的脚本拿到 0 帧连拍并整批假 FAIL
    /// （实测 `repro_tremble far/many` 只有 0~1 帧、`verify_adaptive` 报「未观察到降档」）。
    /// 唯一不设它的是 `tools/verify_occlusion.py` —— 它就是测这条停产门的。
    pub no_fs_hide: bool,
}

fn flag(name: &str) -> bool {
    std::env::var(name).ok().as_deref() == Some("1")
}

fn num<T: std::str::FromStr>(name: &str) -> Option<T> {
    std::env::var(name).ok().and_then(|v| v.parse::<T>().ok())
}

impl EnvOverrides {
    /// 读取一次进程环境。只应在引擎启动时调用。
    pub(crate) fn load() -> Self {
        let sim_backdrop = std::env::var("GLASS_SIM_BACKDROP").ok().and_then(|v| {
            let (wh, hz) = v.split_once('@')?;
            let (w, h) = wh.split_once('x')?;
            Some((
                w.parse::<i32>().ok()?,
                h.parse::<i32>().ok()?,
                hz.parse::<f64>().ok()?.max(1.0),
            ))
        });
        let cursor_at = std::env::var("GLASS_CURSOR_AT").ok().and_then(|v| {
            let (x, y) = v.split_once(',')?;
            Some([x.parse::<f32>().ok()?, y.parse::<f32>().ok()?])
        });
        Self {
            metrics: flag("GLASS_METRICS"),
            default_block: flag("GLASS_DEFAULT_BLOCK"),
            many: flag("GLASS_MANY"),
            spread: flag("GLASS_SPREAD"),
            far_grid: flag("GLASS_FAR_GRID"),
            extreme_spec: flag("GLASS_EXTREME_SPEC"),
            sim_busy: num::<f64>("GLASS_SIM_BUSY")
                .map(|ms| Duration::from_secs_f64(ms.max(0.0) / 1000.0))
                .unwrap_or_default(),
            sim_backdrop,
            monitor_limit: num::<usize>("GLASS_MONITORS"),
            verify_pass_a: flag("GLASS_VERIFY_PASS_A"),
            internal_shot: std::env::var("GLASS_INTERNAL_SHOT").ok(),
            cursor_at,
            no_adaptive: std::env::var("GLASS_ADAPTIVE").ok().as_deref() == Some("0"),
            no_occlusion: std::env::var("GLASS_OCCLUSION").ok().as_deref() == Some("0"),
            no_fs_hide: std::env::var("GLASS_FS_HIDE").ok().as_deref() == Some("0"),
        }
    }

    /// `GLASS_SIM_BUSY` 的微秒数（热路径直接比 0）。
    #[must_use]
    pub(crate) fn sim_busy_us(&self) -> u128 {
        self.sim_busy.as_micros()
    }
}

#[cfg(test)]
mod tests {
    use super::EnvOverrides;

    #[test]
    fn default_is_everything_off() {
        let e = EnvOverrides::default();
        assert!(!e.metrics && !e.many && !e.spread && !e.far_grid);
        assert!(!e.default_block);
        assert!(!e.extreme_spec && !e.verify_pass_a && !e.no_adaptive);
        assert!(!e.no_occlusion, "遮挡剔除的取证后门默认必须是「不关」");
        assert!(!e.no_fs_hide, "全屏隐藏检测的取证后门默认必须是「不关」");
        assert_eq!(e.sim_busy_us(), 0);
        assert!(e.sim_backdrop.is_none());
        assert!(e.monitor_limit.is_none());
        assert!(e.internal_shot.is_none());
        assert!(e.cursor_at.is_none());
    }

    #[test]
    fn load_does_not_panic_without_environment() {
        // 只验证「解析本身健壮」：具体值取决于调用方的环境，不做断言。
        let _ = EnvOverrides::load();
    }
}
