import io, os

path = os.path.join(os.getcwd(), "crates", "liquid-verre", "src", "lib.rs")

with io.open(path, "r", encoding="utf-8", newline="") as f:
    content = f.read()

old_use = "pub use api::{\n    CaptureBackend, CaptureConfig, DiagnosticsConfig, Engine, EngineConfig, EngineEvent,\n    EventHandler, GlassMode, GlassStyle, GlassWindow, LiquidParams, LogLevel, MonitorSnapshot,\n    Profile, Rect, ResizeFeedbackOptions, ResizeHandle, ResizeOptions, Shape, TransparentParams,\n    WindowConfig, WindowLevel, WindowSnapshot, ZOrder, EVENT_QUEUE_CAP,\n};"

new_use = """pub use api::{
    CaptureBackend, CaptureConfig, DiagnosticsConfig, Engine, EngineConfig, EngineEvent,
    EventHandler, GlassInputSink, GlassMode, GlassStyle, GlassWindow, InputDecision,
    LiquidParams, LogLevel, MonitorSnapshot, PointerButton, PointerEvent, PointerPhase,
    Profile, Rect, ResizeFeedbackOptions, ResizeHandle, ResizeOptions, Shape,
    TransparentParams, WindowConfig, WindowLevel, WindowSnapshot, ZOrder, EVENT_QUEUE_CAP,
};"""

content = content.replace(old_use, new_use)

with io.open(path, "w", encoding="utf-8", newline="") as f:
    f.write(content)

print("lib.rs updated OK")