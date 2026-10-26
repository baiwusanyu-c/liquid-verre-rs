import io

path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\runtime\mod.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()

old = "use crate::gfx::{Device, GlassRenderer};"
new = "use crate::gfx::{ContentStore, Device, GlassRenderer};"
assert old in content, "import not found"
content = content.replace(old, new, 1)
print("mod.rs: ContentStore import added")

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("Written")
