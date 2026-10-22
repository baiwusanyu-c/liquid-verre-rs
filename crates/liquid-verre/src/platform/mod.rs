//! Win32 平台层：窗口、层级、显示器 / 桌面查询。
//!
//! 这一层是库里 `unsafe` 密度最高的地方，因此**所有** Win32 调用都收在这里，
//! `core/` 保持纯逻辑（可单测），`runtime/` 只做调度。

pub(crate) mod display;
pub(crate) mod level;
pub(crate) mod occlusion;
pub(crate) mod window;
