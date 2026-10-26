import io, os

path = os.path.join(os.getcwd(), "crates", "liquid-verre", "src", "api", "handle.rs")

with io.open(path, "r", encoding="utf-8", newline="") as f:
    content = f.read()

# 1. Add import for SinkRef
old_import = "use crate::api::config::{"
new_import = "use crate::api::input::SinkRef;\nuse crate::api::config::{"
content = content.replace(old_import, new_import)

# 2. Add Command::SetInputSink at the end of the enum (before the closing brace)
old_end = "    SetMovable { id: u32, movable: bool },\n}"
new_end = "    SetMovable { id: u32, movable: bool },\n    /// 设置输入路由 sink。None = 移除，恢复库默认行为。\n    SetInputSink { id: u32, sink: SinkRef },\n}"
content = content.replace(old_end, new_end)

# 3. Add set_input_sink method on GlassWindow
# Find the last setter method and add after it
old_setter = """    /// 只改缩放总开关。
    pub fn set_resizable(&self, enabled: bool) {
        self.shared.push(Command::SetResizable {
            id: self.id,
            enabled,
        });
    }"""

new_setter = """    /// 只改缩放总开关。
    pub fn set_resizable(&self, enabled: bool) {
        self.shared.push(Command::SetResizable {
            id: self.id,
            enabled,
        });
    }

    /// 设置输入路由 sink。
    ///
    /// 传 `None` = 移除 sink，恢复库默认行为（按压块内 = 移动会话）。
    /// sink 在渲染线程同步调用，契约见 [`GlassInputSink`]。
    pub fn set_input_sink(&self, sink: Option<std::sync::Arc<dyn crate::api::input::GlassInputSink>>) {
        self.shared.push(Command::SetInputSink {
            id: self.id,
            sink: SinkRef(sink),
        });
    }"""

content = content.replace(old_setter, new_setter)

with io.open(path, "w", encoding="utf-8", newline="") as f:
    f.write(content)

print("handle.rs updated OK")