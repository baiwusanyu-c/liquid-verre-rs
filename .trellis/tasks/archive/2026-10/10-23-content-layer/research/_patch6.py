import io

path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\runtime\app.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()

# Add import
old_import = "use crate::api::handle::{Command, MonitorSnapshot, Shared, Snapshot, WindowSnapshot};"
new_import = "use crate::api::handle::{Command, MonitorSnapshot, Shared, Snapshot, WindowSnapshot};\nuse crate::gfx::ContentStore;"
assert old_import in content, "import not found"
content = content.replace(old_import, new_import, 1)
print("app.rs: import added")

# Add field after block_sinks
old_field = "    pub block_sinks: Vec<(u32, SinkRef)>,\n    /// cmd_buf reuse"
new_field = "    pub block_sinks: Vec<(u32, SinkRef)>,\n    pub content_store: ContentStore,\n    /// cmd_buf reuse"
assert old_field in content, "field not found"
content = content.replace(old_field, new_field, 1)
print("app.rs: content_store field added")

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("Written")
