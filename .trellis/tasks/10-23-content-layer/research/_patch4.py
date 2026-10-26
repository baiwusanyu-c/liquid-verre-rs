import io

path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\runtime\commands.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()

# Add UpdateContent/ClearContent handlers after SetInputSink
old = """        Command::SetInputSink { id, sink } => {
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

new = """        Command::SetInputSink { id, sink } => {
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
        Command::UpdateContent {
            id,
            w_phys,
            h_phys,
            buffer,
            dirty: _dirty,
        } => {
            // Content store update triggers a render request; the block's existing
            // dirty/influence-box mechanism is reused (C4: unified invalidation).
            app.content_store
                .update(&app.gfx.device, &app.gfx.ctx, id, w_phys, h_phys, &buffer, None);
            // Mark the block's monitor region dirty so it gets redrawn.
            if let Some(b) = app.manager.get(id) {
                let mi = b.monitor_index;
                if let Some(m) = app.monitors.get_mut(mi) {
                    m.region_dirty = true;
                }
            }
            app.shared.request_render();
            true
        }
        Command::ClearContent { id } => {
            app.content_store.clear(id);
            false
        }
    }
}"""

assert old in content, "old block not found"
content = content.replace(old, new, 1)
print("commands.rs: Patch 1 OK")

# Add content_store.clear(id) in destroy function
old_destroy = """    if app.drag.as_ref().map(|d| d.id()) == Some(id) {
        app.drag = None;
        app.press = 0.0;
    }
    for m in app.monitors.iter_mut() {
        m.region_dirty = true;
    }"""
new_destroy = """    if app.drag.as_ref().map(|d| d.id()) == Some(id) {
        app.drag = None;
        app.press = 0.0;
    }
    app.content_store.clear(id);
    for m in app.monitors.iter_mut() {
        m.region_dirty = true;
    }"""
assert old_destroy in content, "old_destroy not found"
content = content.replace(old_destroy, new_destroy, 1)
print("commands.rs: Patch 2 (destroy) OK")

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("Written")
