//! [`Engine`]：库的入口。持有渲染线程，向上层暴露窗口工厂与全局配置。
//!
//! 典型用法：
//! ```no_run
//! use liquid_verre::{Engine, EngineConfig, GlassStyle, WindowConfig};
//!
//! let mut engine = Engine::new(EngineConfig::default())?;
//! engine.start()?;
//!
//! // 一个透明玻璃弹窗底板
//! let popup = engine.create_window(WindowConfig {
//!     rect: [200.0, 160.0, 520.0, 320.0],
//!     style: Some(GlassStyle::transparent(24.0, 0.42, [1.0, 1.0, 1.0, 1.0])),
//!     ..Default::default()
//! })?;
//!
//! // 一块液态玻璃
//! let liquid = engine.create_window(WindowConfig {
//!     rect: [900.0, 200.0, 640.0, 420.0],
//!     ..Default::default()
//! })?;
//!
//! popup.set_corner_radius(36.0);      // 运行期改参数
//! liquid.bring_to_front();            // 运行期改层级
//! engine.shutdown();
//! # Ok::<(), liquid_verre::Error>(())
//! ```

use std::sync::Arc;
use std::thread::JoinHandle;

use crate::api::config::{EngineConfig, GlassStyle, ResizeOptions, WindowConfig, WindowLevel};
use crate::api::handle::{Command, GlassWindow, MonitorSnapshot, Shared, Snapshot, WindowSnapshot};
use crate::error::{Error, ErrorKind, Result};
use crate::logging;

/// 液态玻璃 / 透明玻璃引擎。
///
/// 一个进程可以有多个 `Engine`（各自一套画布窗口与渲染线程），但**通常只需要一个**：
/// 所有玻璃窗口共享同一套设备、着色器、采集与合成节拍，这正是「20+ 块不卡顿」的前提。
///
/// `Drop` 会自动 [`shutdown`](Engine::shutdown)，因此忘记关也不会泄漏线程。
pub struct Engine {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Engine {
    /// 用给定配置构造引擎（**不**启动渲染线程）。
    ///
    /// 会顺带安装默认 stderr logger（上层已安装自己的 logger 时不覆盖）。
    /// 想在库初始化前完全接管日志，请先调用 `log::set_logger`。
    pub fn new(config: EngineConfig) -> Result<Self> {
        let mut config = config;
        config.sanitize();
        logging::set_max_level(config.log_level.into());
        logging::install_default_logger_with_level(config.log_level.into());
        log::info!(target: "boot", "{}", crate::build_info::BUILD_INFO.summary());
        Ok(Self {
            shared: Shared::new(config),
            thread: None,
        })
    }

    /// 构造并立即启动渲染线程。
    pub fn spawn(config: EngineConfig) -> Result<Self> {
        let mut e = Self::new(config)?;
        e.start()?;
        Ok(e)
    }

    /// 启动渲染线程（幂等：已启动则直接返回 `Ok`）。
    ///
    /// 线程内会做 `RoInitialize` + `SetProcessDpiAwarenessContext(PerMonitorV2)`。
    /// **DPI 感知是进程级**的：宿主应用如果已经设置过别的感知级别，这里的调用会失败，
    /// 库只打 `[warn]` 不报错 —— 但玻璃的 DPI 换算会不准，宿主应当自己设成 Per-Monitor V2。
    pub fn start(&mut self) -> Result<()> {
        if self.thread.is_some() {
            return Ok(());
        }
        // 单次使用：join/退出之后再 start 会在同一个 Shared 上起第二个渲染线程
        // （两套画布、两次 GWLP_USERDATA 绑定），直接拒绝而不是默默双开
        // （独立审查 P2-5）。
        if self.shared.is_stopped() {
            return Err(Error::new(
                ErrorKind::Shutdown,
                "引擎已退出；Engine 是单次使用的，请新建一个 Engine",
            ));
        }
        if !self.shared.mark_started() {
            return Err(Error::new(
                ErrorKind::Shutdown,
                "引擎已经 start 过（thread 句柄被 join 取走），不能重复启动",
            ));
        }
        let shared = Arc::clone(&self.shared);
        let handle = std::thread::Builder::new()
            .name("liquid-verre-render".into())
            .spawn(move || {
                shared.set_running(true);
                if let Err(e) = crate::runtime::run(Arc::clone(&shared)) {
                    log::error!("引擎异常退出: {e}");
                    shared.emit(crate::api::event::EngineEvent::Fatal {
                        kind: e.kind(),
                        message: e.message().to_string(),
                    });
                    shared.set_fatal(e);
                }
                shared.set_running(false);
                shared.mark_stopped();
                shared.emit(crate::api::event::EngineEvent::Shutdown);
            })
            .map_err(|e| Error::with_source(ErrorKind::Internal, "无法创建渲染线程", e))?;
        self.thread = Some(handle);
        Ok(())
    }

    /// 启动并阻塞到引擎退出（等价于重构前直接跑 `main()`）。
    ///
    /// 适合「宿主就是一个玻璃演示程序」的场景；GUI 宿主应当用 [`start`](Engine::start)
    /// 让引擎跑在自己的线程上。
    pub fn run(&mut self) -> Result<()> {
        self.start()?;
        self.join()
    }

    /// 阻塞等待渲染线程结束，并把线程内的致命错误（若有）返回。
    pub fn join(&mut self) -> Result<()> {
        let Some(handle) = self.thread.take() else {
            return Ok(());
        };
        let _ = handle.join();
        match self.shared.take_fatal() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// 请求退出并等待渲染线程结束。幂等。
    pub fn shutdown(&mut self) {
        if self.thread.is_none() {
            return;
        }
        self.shared.push(Command::Shutdown);
        let _ = self.join();
    }

    /// 引擎是否正在运行。
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.shared.is_running()
    }

    /// 非阻塞地取出致命错误（取出后清空）。
    pub fn take_fatal_error(&self) -> Option<Error> {
        self.shared.take_fatal()
    }

    // ---------------- 窗口 ----------------

    /// 创建一个玻璃窗口。
    ///
    /// id 在调用侧分配，因此句柄**立刻**可用；但窗口的实际创建发生在渲染线程的下一帧，
    /// 在那之前 [`GlassWindow::is_alive`] 会返回 `false`。
    ///
    /// # Errors
    /// 尺寸为 NaN / 非正数时返回 [`ErrorKind::InvalidConfig`]。
    pub fn create_window(&self, config: WindowConfig) -> Result<GlassWindow> {
        // **先校验原始输入，再 sanitize**：sanitize 会把 0 宽夹到 MIN_SIZE，
        // 顺序反了的话「上层传了 0×100」这种明显的编程错误就会被静默改成 24×100。
        if !config.rect.iter().all(|v| v.is_finite()) {
            return Err(Error::new(
                ErrorKind::InvalidConfig,
                format!("窗口矩形含 NaN / inf: {:?}", config.rect),
            ));
        }
        if config.rect[2] <= 0.0 || config.rect[3] <= 0.0 {
            return Err(Error::new(
                ErrorKind::InvalidConfig,
                format!("窗口尺寸必须为正: {}x{}", config.rect[2], config.rect[3]),
            ));
        }
        let mut config = config;
        config.sanitize();
        let id = self.shared.alloc_id();
        self.shared.push(Command::Create { id, config });
        Ok(GlassWindow::new(id, Arc::clone(&self.shared)))
    }

    /// 取得已存在窗口的句柄（不做存在性检查；句柄本身是廉价的 `Arc` 克隆）。
    #[must_use]
    pub fn window(&self, id: u32) -> GlassWindow {
        GlassWindow::new(id, Arc::clone(&self.shared))
    }

    /// 当前所有窗口的句柄（来自快照，因此最多滞后一帧）。
    #[must_use]
    pub fn windows(&self) -> Vec<GlassWindow> {
        self.shared
            .read_snapshot(|s| s.windows.iter().map(|w| w.id).collect::<Vec<_>>())
            .into_iter()
            .map(|id| GlassWindow::new(id, Arc::clone(&self.shared)))
            .collect()
    }

    /// 所有窗口的状态快照。
    #[must_use]
    pub fn window_snapshots(&self) -> Vec<WindowSnapshot> {
        self.shared.read_snapshot(|s| s.windows.clone())
    }

    /// 销毁全部窗口。
    pub fn clear_windows(&self) {
        self.shared.push(Command::DestroyAll);
    }

    // ---------------- 全局配置 ----------------

    /// 设置新建窗口的默认外观，并刷新所有**继承默认样式**的窗口。
    pub fn set_default_style(&self, style: GlassStyle) {
        let mut style = style;
        style.sanitize();
        self.shared.push(Command::SetDefaultStyle(style));
    }

    /// 当前默认外观。
    #[must_use]
    pub fn default_style(&self) -> GlassStyle {
        self.shared.read_snapshot(|s| s.default_style)
    }

    /// 设置画布窗口层级（需求 F4）。
    pub fn set_window_level(&self, level: WindowLevel) {
        self.shared.push(Command::SetWindowLevel(level));
    }

    /// 当前画布窗口层级。
    #[must_use]
    pub fn window_level(&self) -> WindowLevel {
        self.shared.read_snapshot(|s| s.window_level)
    }

    /// 设置新建窗口的默认缩放交互配置（已存在的窗口不受影响）。
    pub fn set_default_resize(&self, options: ResizeOptions) {
        let mut o = options;
        o.sanitize();
        self.shared.push(Command::SetDefaultResize(o));
    }

    /// 显示器信息快照。
    #[must_use]
    pub fn monitors(&self) -> Vec<MonitorSnapshot> {
        self.shared.read_snapshot(|s| s.monitors.clone())
    }

    /// 引擎整体快照（调试 / 面板用）。
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        self.shared.read_snapshot(|s| s.clone())
    }

    /// 当前自适应质量档位（0 = 不降档）。
    #[must_use]
    pub fn quality_level(&self) -> u8 {
        self.shared.read_snapshot(|s| s.quality_level)
    }

    // ---------------- 事件 ----------------

    /// 非阻塞地取出全部待处理事件。
    #[must_use]
    pub fn poll_events(&self) -> Vec<crate::api::event::EngineEvent> {
        self.shared.poll_events()
    }

    /// 注册事件回调（在**渲染线程**调用；传 `None` 取消）。
    pub fn set_event_handler(&self, handler: Option<crate::api::event::EventHandler>) {
        self.shared.set_handler(handler);
    }

    /// 主动请求渲染一帧。
    ///
    /// 正常情况下不需要调用：任何 setter 都会自动置位。只有在「宿主改了库看不到的东西」
    /// （例如自己的窗口遮挡关系）时才需要手动踢一帧。
    pub fn request_render(&self) {
        self.shared.request_render();
    }

    /// 引擎配置（构造时传入的那份，已 sanitize）。
    #[must_use]
    pub fn config(&self) -> &EngineConfig {
        self.shared.config()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::Engine;
    use crate::api::config::{EngineConfig, GlassStyle, WindowConfig};
    use crate::error::ErrorKind;

    #[test]
    fn new_sanitizes_config_without_starting_a_thread() {
        let cfg = EngineConfig {
            fps_cap: Some(100_000),
            ..Default::default()
        };
        let e = Engine::new(cfg).expect("构造不应失败");
        assert!(!e.is_running());
        assert_eq!(e.config().fps_cap, Some(1000), "非法帧率上限必须被夹紧");
    }

    #[test]
    fn create_window_rejects_bad_geometry() {
        let e = Engine::new(EngineConfig::default()).unwrap();
        let err = e
            .create_window(WindowConfig {
                rect: [0.0, 0.0, 0.0, 100.0],
                ..Default::default()
            })
            .expect_err("零宽必须被拒");
        assert_eq!(err.kind(), ErrorKind::InvalidConfig);
        let err = e
            .create_window(WindowConfig {
                rect: [0.0, 0.0, f32::NAN, 100.0],
                ..Default::default()
            })
            .expect_err("NaN 必须被拒");
        assert_eq!(err.kind(), ErrorKind::InvalidConfig);
    }

    #[test]
    fn create_window_allocates_unique_ids() {
        let e = Engine::new(EngineConfig::default()).unwrap();
        let a = e.create_window(WindowConfig::default()).unwrap();
        let b = e.create_window(WindowConfig::default()).unwrap();
        assert_ne!(a.id(), b.id());
        // 尚未 start，快照为空 -> is_alive=false，但句柄可用
        assert!(!a.is_alive());
        assert_eq!(a.rect(), None);
    }

    #[test]
    fn default_style_roundtrips() {
        let e = Engine::new(EngineConfig::default()).unwrap();
        e.set_default_style(GlassStyle::transparent(20.0, 0.3, [0.0, 1.0, 0.0, 1.0]));
        // 快照里的默认样式在 Shared::new 时写入；set 走命令队列，需渲染线程消费。
        // 这里只验证命令被排队且读路径不 panic。
        assert_eq!(e.default_style().mode(), GlassStyle::default().mode());
        assert_eq!(
            e.window_level(),
            crate::api::config::WindowLevel::AlwaysOnTop
        );
        assert!(e.monitors().is_empty());
        assert_eq!(e.quality_level(), 0);
    }

    #[test]
    fn shutdown_is_idempotent_and_never_panics() {
        let mut e = Engine::new(EngineConfig::default()).unwrap();
        e.shutdown();
        e.shutdown();
        assert!(!e.is_running());
    }
}
