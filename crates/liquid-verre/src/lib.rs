//! # liquid-verre-rs
//!
//! Windows 桌面**玻璃窗口组件库**：上层应用创建一个玻璃窗口、配置它的外观，
//! 再把自己的 UI 叠在上面，就能得到「液态玻璃弹窗」这类效果。
//!
//! 两种模式（需求 F3）：
//! - **液态玻璃** [`GlassMode::Liquid`]：WGC / DDA 采集桌面 → D3D11 模糊金字塔 →
//!   Snell 折射 + RGB 色散 + Fresnel 高光 + 焦散 + 鼠标点光 → DirectComposition 合成。
//!   约 40 个可调参数，见 [`LiquidParams`]。
//! - **透明玻璃** [`GlassMode::Transparent`]：只有圆角 / 透明度 / 颜色三个参数
//!   （[`TransparentParams`]），不采样模糊金字塔；整屏都是透明块时 Pass A 被完全跳过，
//!   成本远低于液态模式。
//!
//! ## 快速上手
//! ```no_run
//! use liquid_verre::{Engine, EngineConfig, GlassStyle, WindowConfig, WindowLevel};
//!
//! let mut engine = Engine::new(EngineConfig {
//!     window_level: WindowLevel::ClickToFront,
//!     ..Default::default()
//! })?;
//! engine.start()?;
//!
//! let popup = engine.create_window(WindowConfig {
//!     rect: [320.0, 200.0, 560.0, 360.0],
//!     style: Some(GlassStyle::transparent(28.0, 0.40, [1.0, 1.0, 1.0, 1.0])),
//!     ..Default::default()
//! })?;
//!
//! for ev in engine.poll_events() {
//!     println!("{ev:?}");
//! }
//! popup.set_corner_radius(40.0);
//! engine.shutdown();
//! # Ok::<(), liquid_verre::Error>(())
//! ```
//!
//! ## 架构与线程模型
//! - 库内只有**一个渲染线程**：消息泵、桌面采集、Pass A（模糊金字塔）、Pass B（逐块绘制）、
//!   `Present` + DComp `Commit` 全在它上面。这是「20+ 块不卡顿」与「静止桌面 CPU≈0」的前提。
//! - 上层通过 [`GlassWindow`] 句柄写（命令队列，延迟 ≤1 帧）、通过快照读（最终一致）。
//!   **不允许**从上层线程直接触碰 GPU 资源。
//! - 事件与可恢复错误通过 [`EngineEvent`] 上抛（回调或轮询）。
//!
//! ## 层级语义（需求 F4）
//! [`WindowLevel`] 作用于**画布窗口**（每块显示器一个），即整层玻璃相对其它应用窗口的位置；
//! 玻璃块之间的叠放用 [`ZOrder`] / [`GlassWindow::bring_to_front`] 控制。
//!
//! ## 日志与错误
//! - 错误统一为 [`Error`]（带 [`ErrorKind`]，可 `match`；底层 HRESULT 保留在 `source()`）。
//! - 日志走 [`log`] facade，库不自带后端；未安装 logger 时可调用
//!   [`install_default_logger`] 得到与命令行版本一致的 stderr 输出。
//!
//! ## 性能与不变量
//! 渲染管线的 9 条不可破坏不变量、性能阀门阈值与测量方法论见仓库的
//! `BASELINE.md` 与 `.trellis/spec/backend/`。任何改动都要跑
//! `cargo test --release` + `python tools/gate_perf.py` + 8 个回归脚本。

#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod api;

mod build_info;
mod error;
mod logging;

pub(crate) mod capture;
pub(crate) mod core;
pub(crate) mod diag;
pub(crate) mod gfx;
pub(crate) mod input;
pub(crate) mod platform;
pub(crate) mod runtime;

pub use api::{
    CaptureBackend, CaptureConfig, DiagnosticsConfig, Engine, EngineConfig, EngineEvent,
    EventHandler, GlassMode, GlassStyle, GlassWindow, LiquidParams, LogLevel, MonitorSnapshot,
    Profile, Rect, ResizeFeedbackOptions, ResizeHandle, ResizeOptions, Shape, TransparentParams,
    WindowConfig, WindowLevel, WindowSnapshot, ZOrder, EVENT_QUEUE_CAP,
};
pub use build_info::{BuildInfo, BUILD_INFO};
pub use error::{Error, ErrorKind, Result};
pub use logging::{install_default_logger, install_default_logger_with_level};
