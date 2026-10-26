import io
p = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\content_layer.rs"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = "/// 把 buffer 的 [x0..x1)×[y0..y1) 子矩形拷进 GPU 纹理。"
new = "#[allow(clippy::too_many_arguments)]\n/// 把 buffer 的 [x0..x1)×[y0..y1) 子矩形拷进 GPU 纹理。"
assert old in c and "#[allow(clippy::too_many_arguments)]\n///" not in c
c = c.replace(old, new, 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)

p = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\glass_renderer.rs"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = "    fn draw_content_for_block(\n        &self,"
new = "    #[allow(clippy::too_many_arguments)]\n    fn draw_content_for_block(\n        &self,"
assert old in c
c = c.replace(old, new, 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("allow attrs added")
