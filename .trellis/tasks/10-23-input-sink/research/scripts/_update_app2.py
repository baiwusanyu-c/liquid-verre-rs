import io, os

path = os.path.join(os.getcwd(), "crates", "liquid-verre", "src", "runtime", "app.rs")

with io.open(path, "r", encoding="utf-8", newline="") as f:
    content = f.read()

# Add route_state field near press field
old = """    pub press: f32,
    pub drag_velocity: [f32; 2],"""

new = """    pub press: f32,
    /// 输入路由按压归属状态。窗口过程与主循环共享。
    pub route_state: crate::core::routing::RouteState,
    pub drag_velocity: [f32; 2],"""

content = content.replace(old, new)

# Also initialize route_state in App creation. Let me find it.
# The App is created in runtime/mod.rs, not app.rs.
# But there's also initialization in app.rs for the SpawnSpec.
# Let me check...

with io.open(path, "w", encoding="utf-8", newline="") as f:
    f.write(content)

print("app.rs route_state added")