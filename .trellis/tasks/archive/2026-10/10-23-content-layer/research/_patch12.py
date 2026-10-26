import io

path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\shaders.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()

old = 'pub const BORDER_PS: &str = include_str!("../../shaders/border_ps.hlsl");'
new = '''pub const BORDER_PS: &str = include_str!("../../shaders/border_ps.hlsl");
pub const CONTENT_VS: &str = include_str!("../../shaders/content_vs.hlsl");
pub const CONTENT_PS: &str = include_str!("../../shaders/content_ps.hlsl");'''
assert old in content, "BORDER_PS not found"
content = content.replace(old, new, 1)
print("shaders.rs: CONTENT_VS/CONTENT_PS added")

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("Written")
