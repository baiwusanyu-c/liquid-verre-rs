import io, os

path = os.path.join(os.getcwd(), "crates", "liquid-verre", "src", "api", "mod.rs")

with io.open(path, "r", encoding="utf-8", newline="") as f:
    content = f.read()

# Add pub mod input after the existing module declarations
content = content.replace(
    "pub mod config;\npub mod engine;\npub mod event;\npub mod handle;",
    "pub mod config;\npub mod engine;\npub mod event;\npub mod handle;\npub mod input;"
)

# Add re-exports for input types
old_use = "pub use handle::{GlassWindow, MonitorSnapshot, Snapshot, WindowSnapshot};"
new_use = """pub use handle::{GlassWindow, MonitorSnapshot, Snapshot, WindowSnapshot};
pub use input::{GlassInputSink, InputDecision, PointerButton, PointerEvent, PointerPhase};"""

content = content.replace(old_use, new_use)

with io.open(path, "w", encoding="utf-8", newline="") as f:
    f.write(content)

print("mod.rs updated OK")