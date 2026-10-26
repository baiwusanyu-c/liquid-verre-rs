import io, os

path = os.path.join(os.getcwd(), "crates", "liquid-verre", "src", "runtime", "commands.rs")

with io.open(path, "r", encoding="utf-8", newline="") as f:
    content = f.read()

# Add import for SinkRef
old_import = "use crate::api::handle::{Command, StylePatch};"
new_import = "use crate::api::handle::{Command, StylePatch};\nuse crate::api::input::SinkRef;"
content = content.replace(old_import, new_import)

# Add SetInputSink handling before the closing of apply_one's match
# Find the last match arm (SetInteractive) and add SetInputSink after it
old_arm = """        Command::SetInteractive { id, interactive } => {
            let Some(b) = app.manager.get_mut(id) else {
                return warn_missing(app, id, "set_interactive");
            };
            if b.interactive == interactive {
                return false;
            }
            b.interactive = interactive;
            app.snapshot_dirty = true;
            false
        }
    }
}"""

new_arm = """        Command::SetInteractive { id, interactive } => {
            let Some(b) = app.manager.get_mut(id) else {
                return warn_missing(app, id, "set_interactive");
            };
            if b.interactive == interactive {
                return false;
            }
            b.interactive = interactive;
            app.snapshot_dirty = true;
            false
        }
        Command::SetInputSink { id, sink } => {
            // 帧首消费：写入或移除 sink 槽位。
            // 不触发重画（sink 不影响渲染管线），不设 snapshot_dirty。
            if let Some(pos) = app.block_sinks.iter().position(|(bid, _)| *bid == id) {
                if sink.0.is_none() {
                    app.block_sinks.remove(pos);
                } else {
                    app.block_sinks[pos] = (id, sink);
                }
            } else if sink.0.is_some() {
                app.block_sinks.push((id, sink));
            }
            false
        }
    }
}"""

content = content.replace(old_arm, new_arm)

with io.open(path, "w", encoding="utf-8", newline="") as f:
    f.write(content)

print("commands.rs updated OK")