import io, os

path = os.path.join(os.getcwd(), "crates", "liquid-verre", "src", "platform", "window.rs")

with io.open(path, "r", encoding="utf-8", newline="") as f:
    lines = f.readlines()

# The import lines are 23-30 (0-indexed: 22-29)
# Let me verify
for i in range(22, 31):
    print(f"Line {i+1}: {lines[i].rstrip()}")

# Replace lines 22-29 (0-indexed) with new imports
new_import_lines = [
    "use crate::api::config::WindowLevel;\n",
    "use crate::api::event::EngineEvent;\n",
    "use crate::api::handle::Command;\n",
    "use crate::api::input::{PointerButton, PointerEvent, PointerPhase};\n",
    "use crate::core::handle::ResizeEdges;\n",
    "use crate::core::routing::{self, PhaseKind, RouteAction, RouteInput, RouteState, SinkDecision};\n",
    "use crate::error::{Error, ErrorKind, Result};\n",
    "use crate::input::{DragSession, MoveState, ResizeState};\n",
    "use crate::platform::{display::MonitorInfo, level};\n",
    "use crate::runtime::{app::App, monitor::MonitorRt};\n",
]

new_lines = lines[:22] + new_import_lines + lines[30:]

with io.open(path, "w", encoding="utf-8", newline="") as f:
    f.writelines(new_lines)

print("imports updated")