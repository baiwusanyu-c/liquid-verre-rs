import io

# 1. Add ContentCb struct in glass_renderer.rs
path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\glass_renderer.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()

# Add ContentCb after BorderCb
old_bcb = """struct BorderCb {
    v: [f32; BORDER_CB_FLOATS],
}"""
new_bcb = """struct BorderCb {
    v: [f32; BORDER_CB_FLOATS],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ContentCb {
    v: [f32; CONTENT_CB_FLOATS],
}"""
assert old_bcb in content, "BorderCb not found"
content = content.replace(old_bcb, new_bcb, 1)
print("ContentCb struct added")

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)

# 2. Wire up compositor to call draw_contents
path2 = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\compositor.rs"
with io.open(path2, "r", encoding="utf-8") as f:
    c2 = f.read()

# Find draw_blocks call and draw_borders call, insert draw_contents between them
old_comp = """            glass.draw_borders(
                &gfx.ctx,"""
new_comp = """            glass.draw_contents(
                &gfx.ctx,
                &crate::gfx::glass_renderer::BorderTarget {
                    rtv: &self.rt.rtv,
                    w: self.rt.w,
                    h: self.rt.h,
                    canvas: [self.width as f32, self.height as f32],
                },
                blocks,
                order,
                blocks_content,
            );
            glass.draw_borders(
                &gfx.ctx,"""
assert old_comp in c2, "draw_borders not found in compositor"
c2 = c2.replace(old_comp, new_comp, 1)
print("compositor: draw_contents wired")

with io.open(path2, "w", encoding="utf-8", newline="\n") as f:
    f.write(c2)

print("All done")
