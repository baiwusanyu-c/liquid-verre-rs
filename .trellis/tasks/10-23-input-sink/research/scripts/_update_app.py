import io, os

path = os.path.join(os.getcwd(), "crates", "liquid-verre", "src", "runtime", "app.rs")

with io.open(path, "r", encoding="utf-8", newline="") as f:
    content = f.read()

# Add import for SinkRef
old_import = "use crate::api::handle::{Command, MonitorSnapshot, Shared, Snapshot, WindowSnapshot};"
new_import = "use crate::api::handle::{Command, MonitorSnapshot, Shared, Snapshot, WindowSnapshot};\nuse crate::api::input::SinkRef;"
if old_import not in content:
    # Try alternate
    old_import = "use crate::api::handle::{"
    new_import = "use crate::api::input::SinkRef;\nuse crate::api::handle::{"
content = content.replace(old_import, new_import, 1)

# Add block_sinks field after style_cache
old_field = "    pub style_cache: StyleCache,\n    /// 命令缓冲"
new_field = "    pub style_cache: StyleCache,\n    /// 每块的输入路由 sink。id -> SinkRef；setter 走命令入队、帧首消费。\n    pub block_sinks: Vec<(u32, SinkRef)>,\n    /// 命令缓冲"
content = content.replace(old_field, new_field)

# Add helper method: find sink for a block id
# Find a good insertion point - after read_cursor
old_method = """    /// 窗口过程与主循环都必须走这里，不能再各自调 `cursor_pos()` ——"""

# Add after read_cursor method. Let me find it.
# read_cursor method is already there. Add sink_for method after it.

# Let me find the end of read_cursor
old_end_of_read_cursor = """                let p = crate::input::cursor_pos();
                [p.x as f32, p.y as f32]
            }
        }
    }"""

new_end = """                let p = crate::input::cursor_pos();
                [p.x as f32, p.y as f32]
            }
        }
    }

    /// 查块的输入 sink（零分配：只克隆 Arc 引用）。
    #[inline]
    pub(crate) fn sink_for(&self, id: u32) -> Option<std::sync::Arc<dyn crate::api::input::GlassInputSink>> {
        self.block_sinks
            .iter()
            .find(|(bid, _)| *bid == id)
            .and_then(|(_, sr)| sr.0.clone())
    }"""

content = content.replace(old_end_of_read_cursor, new_end)

with io.open(path, "w", encoding="utf-8", newline="") as f:
    f.write(content)

print("app.rs updated OK")