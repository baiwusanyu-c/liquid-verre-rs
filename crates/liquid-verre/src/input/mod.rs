//! 输入层：光标轮询（零钩子，方案 6.3）+ 拖拽 / 缩放状态机。

pub(crate) mod cursor;
pub(crate) mod drag;

pub(crate) use cursor::cursor_pos;
pub(crate) use drag::{DragSession, MoveState, ResizeState};
