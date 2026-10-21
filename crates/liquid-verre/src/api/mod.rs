//! 公共 API 面：**这是库唯一对上层暴露的模块**。
//!
//! 分层：
//! - [`Engine`]：进程内一个（或多个）引擎，拥有渲染线程与全部 GPU 资源；
//! - [`GlassWindow`]：一个玻璃窗口的句柄，`Clone + Send + Sync`，setter 异步、getter 读快照；
//! - [`EngineConfig`] / [`WindowConfig`] / [`GlassStyle`]：全部可调参数（需求 F1）；
//! - [`EngineEvent`]：运行期状态与可恢复错误（工程需求 2）。

pub mod config;
pub mod engine;
pub mod event;
pub mod handle;

pub use config::{
    CaptureBackend, CaptureConfig, DiagnosticsConfig, EngineConfig, GlassMode, GlassStyle,
    LogLevel, Rect, ResizeFeedbackOptions, ResizeOptions, TransparentParams, WindowConfig,
    WindowLevel, ZOrder,
};
pub use engine::Engine;
pub use event::{EngineEvent, EventHandler, ResizeHandle, EVENT_QUEUE_CAP};
pub use handle::{GlassWindow, MonitorSnapshot, Snapshot, WindowSnapshot};

/// 液态玻璃的全参数表（= 内部 `core::params::LiquidParams`）。
pub use crate::core::params::{LiquidParams, Profile, Shape};
