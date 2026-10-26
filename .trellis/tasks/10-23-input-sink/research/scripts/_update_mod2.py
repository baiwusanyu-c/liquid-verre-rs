import io, os

path = os.path.join(os.getcwd(), "crates", "liquid-verre", "src", "runtime", "mod.rs")

with io.open(path, "r", encoding="utf-8", newline="") as f:
    content = f.read()

# Add route_state initialization after press: 0.0, (twice - two App construction sites)
old = """        press: 0.0,
        drag_velocity: [0.0, 0.0],"""

new = """        press: 0.0,
        route_state: crate::core::routing::RouteState::Idle,
        drag_velocity: [0.0, 0.0],"""

# Replace first occurrence
content = content.replace(old, new, 1)
# Replace second occurrence  
content = content.replace(old, new, 1)

with io.open(path, "w", encoding="utf-8", newline="") as f:
    f.write(content)

print("mod.rs route_state init added")