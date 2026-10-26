import io, os

ROOT = os.path.join(os.getcwd(), "crates", "liquid-verre", "src")

# ============================================================
# 1. api/handle.rs
# ============================================================
path = os.path.join(ROOT, "api", "handle.rs")
with io.open(path, "r", encoding="utf-8", newline="") as f:
    lines = f.readlines()

# 1a: add import after api::config
for i, line in enumerate(lines):
    if "use crate::api::config::{" in line:
        lines.insert(i, "use crate::api::input::SinkRef;\n")
        break

# 1b: add SetInputSink variant (find "SetMovable" line and add after)
for i, line in enumerate(lines):
    if "SetMovable { id: u32, movable: bool }," in line:
        lines.insert(i + 1, "    SetInputSink { id: u32, sink: SinkRef },\n")
        break

# 1c: add set_input_sink method (find "set_resizable" method, add after it closes)
for i, line in enumerate(lines):
    if "pub fn set_resizable(&self, enabled: bool)" in line:
        # Find the end of this method
        depth = 0
        started = False
        for j in range(i, len(lines)):
            depth += lines[j].count("{") - lines[j].count("}")
            if "{" in lines[j]:
                started = True
            if started and depth == 0:
                # Insert after this line
                insert_at = j + 1
                method = [
                    "\n",
                    "    /// Set input routing sink.\n",
                    "    pub fn set_input_sink(&self, sink: Option<std::sync::Arc<dyn crate::api::input::GlassInputSink>>) {\n",
                    "        self.shared.push(Command::SetInputSink {\n",
                    "            id: self.id,\n",
                    "            sink: SinkRef(sink),\n",
                    "        });\n",
                    "    }\n",
                ]
                for item in reversed(method):
                    lines.insert(insert_at, item)
                break
        break

with io.open(path, "w", encoding="utf-8", newline="") as f:
    f.writelines(lines)
print("handle.rs done")

# ============================================================
# 2. runtime/commands.rs
# ============================================================
path = os.path.join(ROOT, "runtime", "commands.rs")
with io.open(path, "r", encoding="utf-8", newline="") as f:
    lines = f.readlines()

# 2a: add import after Command import
for i, line in enumerate(lines):
    if "use crate::api::handle::{Command, StylePatch};" in line:
        lines.insert(i + 1, "use crate::api::input::SinkRef;\n")
        break

# 2b: add SetInputSink handler. Find SetInteractive arm end.
# SetInteractive arm pattern: "Command::SetInteractive ... false\n        }\n    }\n}\n\nfn destroy"
for i, line in enumerate(lines):
    if "Command::SetInteractive { id, interactive }" in line:
        # Find the "false" line within this arm
        for j in range(i + 1, min(i + 20, len(lines))):
            if lines[j].strip() == "false":
                # Insert after the '}' that follows (this is the arm closing)
                # Actually, insert before the closing braces of apply_one
                # The pattern is:
                #         }
                #     }
                # }
                # 
                # fn destroy
                pass
        break

# Better approach: find the closing of apply_one function
for i, line in enumerate(lines):
    if i > 200 and line.strip() == "}" and "fn apply_one" in "".join(lines[max(0,i-30):i]):
        # Check if next is blank then fn destroy
        if i+2 < len(lines) and "fn destroy" in lines[i+2]:
            # This is apply_one's closing }. Insert before it.
            insert_at = i
            handler = [
                "        Command::SetInputSink { id, sink } => {\n",
                "            if let Some(pos) = app.block_sinks.iter().position(|(bid, _)| *bid == id) {\n",
                "                if sink.0.is_none() {\n",
                "                    app.block_sinks.remove(pos);\n",
                "                } else {\n",
                "                    app.block_sinks[pos] = (id, sink);\n",
                "                }\n",
                "            } else if sink.0.is_some() {\n",
                "                app.block_sinks.push((id, sink));\n",
                "            }\n",
                "            false\n",
                "        }\n",
            ]
            for item in reversed(handler):
                lines.insert(insert_at, item)
            break

with io.open(path, "w", encoding="utf-8", newline="") as f:
    f.writelines(lines)
print("commands.rs done")

# ============================================================
# 3. runtime/app.rs
# ============================================================
path = os.path.join(ROOT, "runtime", "app.rs")
with io.open(path, "r", encoding="utf-8", newline="") as f:
    lines = f.readlines()

# 3a: add import
for i, line in enumerate(lines):
    if "use crate::api::handle::{" in line:
        lines.insert(i + 1, "use crate::api::input::SinkRef;\n")
        break

# 3b: add route_state after "pub press: f32,"
for i, line in enumerate(lines):
    if line.strip() == "pub press: f32," and i > 50:
        lines.insert(i + 1, "    pub route_state: crate::core::routing::RouteState,\n")
        break

# 3c: add block_sinks after style_cache
for i, line in enumerate(lines):
    if "pub style_cache: StyleCache," in line:
        lines.insert(i + 1, "    pub block_sinks: Vec<(u32, SinkRef)>,\n")
        break

# 3d: add impl App with sink_for before DragSessionRef
for i, line in enumerate(lines):
    if "pub(crate) type DragSessionRef" in line:
        impl_block = [
            "\n",
            "impl App {\n",
            "    #[inline]\n",
            "    pub(crate) fn sink_for(&self, id: u32) -> Option<std::sync::Arc<dyn crate::api::input::GlassInputSink>> {\n",
            "        self.block_sinks\n",
            "            .iter()\n",
            "            .find(|(bid, _)| *bid == id)\n",
            "            .and_then(|(_, sr)| sr.0.clone())\n",
            "    }\n",
            "}\n",
        ]
        for item in reversed(impl_block):
            lines.insert(i, item)
        break

with io.open(path, "w", encoding="utf-8", newline="") as f:
    f.writelines(lines)
print("app.rs done")

# ============================================================
# 4. runtime/mod.rs
# ============================================================
path = os.path.join(ROOT, "runtime", "mod.rs")
with io.open(path, "r", encoding="utf-8", newline="") as f:
    lines = f.readlines()

# 4a: add route_state after "press: 0.0,"
count = 0
for i, line in enumerate(lines):
    if line.strip() == "press: 0.0,":
        lines.insert(i + 1, "        route_state: crate::core::routing::RouteState::Idle,\n")
        count += 1

# 4b: add block_sinks after style_cache init
for i, line in enumerate(lines):
    if "style_cache: StyleCache::default()," in line:
        lines.insert(i + 1, "        block_sinks: Vec::new(),\n")
        break

with io.open(path, "w", encoding="utf-8", newline="") as f:
    f.writelines(lines)
print(f"mod.rs done (route_state added {count} times)")

print("\nAll files updated!")