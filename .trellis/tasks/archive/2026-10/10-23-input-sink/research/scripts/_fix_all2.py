import io, os

ROOT = os.path.join(os.getcwd(), "crates", "liquid-verre", "src")

# ============================================================
# Fix runtime/commands.rs - insert SetInputSink handler before line 290
# ============================================================
path = os.path.join(ROOT, "runtime", "commands.rs")
with io.open(path, "r", encoding="utf-8", newline="") as f:
    lines = f.readlines()

# The closing } of apply_one is at line 290 (0-indexed: 289)
# Insert SetInputSink handler before that
insert_at = 289  # 0-indexed for line 290
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

with io.open(path, "w", encoding="utf-8", newline="") as f:
    f.writelines(lines)
print("commands.rs: SetInputSink handler added at line", insert_at + 1)

# ============================================================
# Fix runtime/mod.rs - add route_state and block_sinks initializers
# ============================================================
path = os.path.join(ROOT, "runtime", "mod.rs")
with io.open(path, "r", encoding="utf-8", newline="") as f:
    lines = f.readlines()

# Find "press: 0.0," in App initialization (first occurrence, ~line 226)
# And find "style_cache: StyleCache::default()," (~line 230+)
# And make sure NOT to add route_state to FrameCtx

press_count = 0
for i, line in enumerate(lines):
    if line.strip() == "press: 0.0,":
        press_count += 1
        if press_count == 1:  # First occurrence = App struct
            lines.insert(i + 1, "        route_state: crate::core::routing::RouteState::Idle,\n")
        # Skip the second occurrence (FrameCtx)

for i, line in enumerate(lines):
    if "style_cache: StyleCache::default()," in line:
        lines.insert(i + 1, "        block_sinks: Vec::new(),\n")
        break

with io.open(path, "w", encoding="utf-8", newline="") as f:
    f.writelines(lines)
print("mod.rs: initializers added")

print("\nDone!")