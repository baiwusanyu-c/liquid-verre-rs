import io, os

path = os.path.join(os.getcwd(), "crates", "liquid-verre", "src", "platform", "window.rs")

with io.open(path, "r", encoding="utf-8", newline="") as f:
    content = f.read()

old_imports = """use crate::api::config::WindowLevel;
use crate::api::event::EngineEvent;
use crate::api::handle::Command;
use crate::core::handle::ResizeEdges;
use crate::error::{Error, ErrorKind, Result};
use crate::input::{DragSession, MoveState, ResizeState};
use crate::platform::{display::MonitorInfo, level};
use crate::runtime::{app::App, monitor::MonitorRt};"""

new_imports = """use crate::api::config::WindowLevel;
use crate::api::event::EngineEvent;
use crate::api::handle::Command;
use crate::api::input::{PointerButton, PointerEvent, PointerPhase};
use crate::core::handle::ResizeEdges;
use crate::core::routing::{self, PhaseKind, RouteAction, RouteInput, RouteState, SinkDecision};
use crate::error::{Error, ErrorKind, Result};
use crate::input::{DragSession, MoveState, ResizeState};
use crate::platform::{display::MonitorInfo, level};
use crate::runtime::{app::App, monitor::MonitorRt};"""

content = content.replace(old_imports, new_imports)

with io.open(path, "w", encoding="utf-8", newline="") as f:
    f.write(content)

print("imports updated OK")