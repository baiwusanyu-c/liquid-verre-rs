import io

# 1. Add content_store to FrameInput in compositor.rs
path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\compositor.rs"
with io.open(path, "r", encoding="utf-8") as f:
    c = f.read()

old_fi = "    pub needs_backdrop: bool,\n}"
new_fi = "    pub needs_backdrop: bool,\n    pub content_store: &'a crate::gfx::content_layer::ContentStore,\n}"
assert old_fi in c, "FrameInput end not found"
c = c.replace(old_fi, new_fi, 1)
print("FrameInput: content_store added")

# 2. Add content_store to the FrameInput destructure in render()
old_dest = "        let FrameInput {\n            blocks,\n            order,\n            cursor_screen,\n            canvas_origin,\n            prev_cursor_canvas,"
new_dest = "        let FrameInput {\n            blocks,\n            order,\n            cursor_screen,\n            canvas_origin,\n            prev_cursor_canvas,\n            content_store,"
assert old_dest in c, "FrameInput destructure not found"
c = c.replace(old_dest, new_dest, 1)
print("FrameInput: content_store destructured")

# 3. Fix the draw_contents call - use content_store not blocks_content
old_dc = "                blocks_content,\n            );"
new_dc = "                content_store,\n            );"
assert old_dc in c, "blocks_content ref not found"
c = c.replace(old_dc, new_dc, 1)
print("draw_contents: uses content_store")

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)

# 4. Add content_store to FrameInput construction in app.rs
path2 = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\runtime\app.rs"
with io.open(path2, "r", encoding="utf-8") as f:
    c2 = f.read()

old_fi2 = "        needs_backdrop: mon.needs_backdrop,\n    };"
new_fi2 = "        needs_backdrop: mon.needs_backdrop,\n        content_store: &app.content_store,\n    };"
assert old_fi2 in c2, "FrameInput construction not found in app.rs"
c2 = c2.replace(old_fi2, new_fi2, 1)
print("app.rs: content_store added to FrameInput")

with io.open(path2, "w", encoding="utf-8", newline="\n") as f:
    f.write(c2)

print("All done")
