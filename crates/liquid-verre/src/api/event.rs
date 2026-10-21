//! 引擎事件：把运行期状态变化 / 可恢复错误上抛给上层应用（工程需求 2）。
//!
//! 两种消费方式，可以并用：
//! 1. [`Engine::set_event_handler`](super::Engine::set_event_handler)：在**渲染线程**同步回调，
//!    适合需要立刻响应的场景（例如缩放结束后同步自己 UI 窗口的位置）。
//!    回调里**不要阻塞、不要重入引擎**（会死锁）；重活请转到自己的线程。
//! 2. [`Engine::poll_events`](super::Engine::poll_events)：从内部有界队列批量取，
//!    适合在宿主自己的 UI 循环里轮询。队列上限 [`EVENT_QUEUE_CAP`]，溢出丢最旧的一条，
//!    并在下一次轮询里通过 [`EngineEvent::EventsDropped`] 告知丢了多少。
//!
//! 可恢复错误（采集失效、独占全屏让路、DPI 变化）走 `Warning` / 专用事件，
//! **不会**让 `Engine` 的方法返回 `Err`；只有不可恢复的初始化失败才是 `Err`。

use std::fmt;

use crate::api::config::{Rect, WindowLevel};
use crate::error::ErrorKind;

/// 事件队列上限。
pub const EVENT_QUEUE_CAP: usize = 256;

/// 缩放把手方向（[`EngineEvent::ResizeStarted`] 用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResizeHandle {
    /// 左边。
    Left,
    /// 右边。
    Right,
    /// 上边。
    Top,
    /// 下边。
    Bottom,
    /// 左上角（圆角）。
    TopLeft,
    /// 右上角（圆角）。
    TopRight,
    /// 左下角（圆角）。
    BottomLeft,
    /// 右下角（圆角）。
    BottomRight,
}

impl fmt::Display for ResizeHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ResizeHandle::Left => "left",
            ResizeHandle::Right => "right",
            ResizeHandle::Top => "top",
            ResizeHandle::Bottom => "bottom",
            ResizeHandle::TopLeft => "top-left",
            ResizeHandle::TopRight => "top-right",
            ResizeHandle::BottomLeft => "bottom-left",
            ResizeHandle::BottomRight => "bottom-right",
        };
        f.write_str(s)
    }
}

/// 引擎事件。
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum EngineEvent {
    /// 引擎已就绪：设备 / 着色器 / 各屏画布与采集都建好了。
    Ready {
        /// 参与渲染的显示器数量。
        monitor_count: usize,
    },
    /// 显示器集合发生变化（分辨率 / 缩放 / 拔插），画布与采集已重建。
    MonitorsChanged {
        /// 重建后的显示器数量。
        monitor_count: usize,
    },
    /// 一个玻璃窗口已创建。
    WindowCreated {
        /// 窗口 id。
        id: u32,
    },
    /// 一个玻璃窗口已销毁。
    WindowDestroyed {
        /// 窗口 id。
        id: u32,
    },
    /// 开始拖动位置。
    MoveStarted {
        /// 窗口 id。
        id: u32,
    },
    /// 拖动位置结束（`rect` 为逻辑像素）。
    MoveEnded {
        /// 窗口 id。
        id: u32,
        /// 结束后的矩形。
        rect: Rect,
    },
    /// 开始缩放（边缘或对角）。
    ResizeStarted {
        /// 窗口 id。
        id: u32,
        /// 抓住的把手。
        handle: ResizeHandle,
    },
    /// 缩放结束（`rect` 为逻辑像素）。
    ResizeEnded {
        /// 窗口 id。
        id: u32,
        /// 结束后的矩形。
        rect: Rect,
    },
    /// 画布窗口层级被应用。
    LevelApplied {
        /// 新层级。
        level: WindowLevel,
    },
    /// 某屏选定的采集后端。
    CaptureBackendSelected {
        /// 显示器下标。
        monitor_index: usize,
        /// 显示器名。
        monitor_name: String,
        /// 后端名（`"dda"` / `"wgc"` / `"synth"`）。
        backend: String,
    },
    /// 某屏采集失效（锁屏 / 独占全屏 / 被其它进程抢占）。库会按退避自动重建。
    CaptureLost {
        /// 显示器下标。
        monitor_index: usize,
        /// 类别。上层按它分支（`error.rs` 的契约：不要匹配字符串）；
        /// `reason` 只用于日志 / 崩溃上报的人类可读文本。
        kind: ErrorKind,
        /// 失效原因（HRESULT 文本）。
        reason: String,
    },
    /// 某屏采集已恢复。
    CaptureRestored {
        /// 显示器下标。
        monitor_index: usize,
    },
    /// 某屏检测到独占全屏应用（画布临时隐藏，方案 R-16）。
    ExclusiveFullscreen {
        /// 显示器下标。
        monitor_index: usize,
        /// `true` = 刚检测到，`false` = 已退出。
        active: bool,
    },
    /// 自适应质量档位变化（方案 12.4.3）。level 0 = 不降档。
    QualityLevelChanged {
        /// 新档位（0–4）。
        level: u8,
    },
    /// 可恢复告警。
    Warning {
        /// 类别。
        kind: ErrorKind,
        /// 说明。
        message: String,
    },
    /// 不可恢复错误：引擎即将退出。
    Fatal {
        /// 类别。
        kind: ErrorKind,
        /// 说明。
        message: String,
    },
    /// 事件队列溢出，丢弃了 `count` 条最旧的事件。
    EventsDropped {
        /// 丢弃条数。
        count: u64,
    },
    /// 引擎已退出。
    Shutdown,
}

/// 事件回调。在**渲染线程**调用，实现方不得阻塞或重入引擎。
pub type EventHandler = std::sync::Arc<dyn Fn(&EngineEvent) + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::{EngineEvent, ResizeHandle, EVENT_QUEUE_CAP};

    #[test]
    fn handle_display_is_kebab_case() {
        assert_eq!(ResizeHandle::TopLeft.to_string(), "top-left");
        assert_eq!(ResizeHandle::Bottom.to_string(), "bottom");
    }

    #[test]
    fn events_are_clone_and_comparable() {
        let e = EngineEvent::ResizeEnded {
            id: 3,
            rect: [1.0, 2.0, 3.0, 4.0],
        };
        assert_eq!(e.clone(), e);
        // 队列上限的意义在于「溢出要能被观测到」，不是常量本身
        let dropped = EngineEvent::EventsDropped {
            count: u64::from(EVENT_QUEUE_CAP as u32),
        };
        assert_ne!(dropped, e);
    }
}
