import io, os

path = os.path.join(os.getcwd(), "crates", "liquid-verre", "src", "api", "input.rs")

with io.open(path, "w", encoding="utf-8", newline="") as f:
    f.write("""//! 输入路由公共类型：指针事件与 GlassInputSink trait。
//!
//! # 线程契约（必读）
//!
//! GlassInputSink::on_pointer 在**渲染线程**（wnd_proc 上下文）同步调用。
//! 实现方必须遵守与 EventHandler 相同的契约：
//!
//! - **微秒级返回**
//! - **不阻塞**
//! - **不重入引擎**
//! - **禁止 DoDragDrop**：OLE 模态消息泵会卡死渲染线程
//!
//! 允许：回调内调用 GlassWindow 的 setter（异步入队，<=1 帧生效）。
//!
//! # 决策顺序（硬规则）
//!
//! 1. 缩放把手带优先级恒高于 sink
//! 2. 块矩形内命中 -> 询问该块 sink
//! 3. 无 sink 的块完全保持 0.4.2 行为

use std::sync::Arc;

/// 指针事件阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointerPhase {
    /// 左键按下。
    Down,
    /// 按键抬起。Some(Right) 表示右键抬起。
    Up(Option<PointerButton>),
    /// 光标移动。
    Move,
    /// 左键双击。不补发 Down，不产生拖拽会话。
    DoubleClick,
    /// 右键按下。
    RightDown,
    /// 滚轮。wheel_delta = delta/120。
    Wheel,
}

/// 指针按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointerButton {
    Left,
    Right,
}

/// 一次指针事件（块内物理像素坐标）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerEvent {
    /// 光标在块矩形内的物理像素坐标。
    pub block_local: [f32; 2],
    /// 事件阶段。
    pub phase: PointerPhase,
    /// 按钮。
    pub button: Option<PointerButton>,
    /// 滚轮增量（仅 Wheel 阶段有效）。
    pub wheel_delta: i32,
}

/// sink 对事件的决策。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputDecision {
    /// 消费事件：不开移动会话、不 SetCapture。
    Consumed,
    /// 交给库启动移动会话。
    DragMove,
    /// 由库按默认行为处理（= DragMove）。
    Ignored,
}

/// 块级输入路由接口。
///
/// # 线程契约
///
/// 所有方法在**渲染线程**同步调用。
pub trait GlassInputSink: Send + Sync {
    /// 指针事件路由到该块。
    ///
    /// - Down 阶段：Consumed = 不开移动会话/不 SetCapture。
    /// - 其余阶段：返回值仅作记录。
    ///
    /// 微秒级返回，不阻塞，不重入引擎。
    /// **禁止在回调内调用 DoDragDrop**。
    fn on_pointer(&self, id: u32, event: &PointerEvent) -> InputDecision;

    /// 光标覆盖：返回 Windows 光标资源 id。
    /// 缩放把手优先级恒高于此方法。默认 None。
    fn cursor_override(&self, _id: u32) -> Option<isize> {
        None
    }
}

/// set_input_sink 的入队负载（内部使用）。
#[derive(Clone)]
pub(crate) struct SinkRef(pub Option<Arc<dyn GlassInputSink>>);

impl std::fmt::Debug for SinkRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Some(_) => write!(f, "Some(GlassInputSink)"),
            None => write!(f, "None"),
        }
    }
}

impl PartialEq for SinkRef {
    fn eq(&self, other: &Self) -> bool {
        self.0.is_some() == other.0.is_some()
    }
}
""")

print("input.rs written OK")