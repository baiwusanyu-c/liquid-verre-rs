import io

path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\content_layer.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()

old = "    /// Upload or replace content for a block. Recreates the texture if dimensions changed.\n    pub fn update("
new = "    /// Upload or replace content for a block. Recreates the texture if dimensions changed.\n    #[allow(clippy::too_many_arguments)]\n    pub fn update("
assert old in content, "update method not found"
content = content.replace(old, new, 1)
print("Added allow(clippy::too_many_arguments)")

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("Written")
