import io
path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\glass_renderer.rs"
with io.open(path, "r", encoding="utf-8") as f:
    c = f.read()

old = "                if let Some(layer) = store.get(block.id) {\n                    self.draw_content_for_block(ctx, block, p.corner_radius, layer);\n                }"
new = "                if let Some(layer) = store.get(block.id) {\n                    self.draw_content_for_block(\n                        ctx,\n                        block,\n                        p.corner_radius,\n                        layer,\n                        canvas_w,\n                        canvas_h,\n                    );\n                }"
assert old in c, "call site not found"
c = c.replace(old, new, 1)

old = "    fn draw_content_for_block(\n        &self,\n        ctx: &ID3D11DeviceContext,\n        block: &DrawBlock,\n        radius: f32,\n        layer: &crate::gfx::content_layer::ContentLayer,\n    ) {"
new = "    fn draw_content_for_block(\n        &self,\n        ctx: &ID3D11DeviceContext,\n        block: &DrawBlock,\n        radius: f32,\n        layer: &crate::gfx::content_layer::ContentLayer,\n        canvas_w: f32,\n        canvas_h: f32,\n    ) {"
assert old in c, "helper sig not found"
c = c.replace(old, new, 1)

old = "        if update_cbuffer(ctx, &self.ccb, &cb).is_err() {\n            return;\n        }\n        unsafe {\n            ctx.PSSetShaderResources(9,"
new = "        set4(&mut cb, 2, canvas_w, canvas_h, 0.0, 0.0);\n        if update_cbuffer(ctx, &self.ccb, &cb).is_err() {\n            return;\n        }\n        unsafe {\n            ctx.PSSetShaderResources(9,"
assert old in c, "cb update not found"
c = c.replace(old, new, 1)

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("helper: c2 canvas size filled")
