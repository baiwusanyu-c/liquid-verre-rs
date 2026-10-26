import io, os
R = r"D:\github-project\liquid-verre-rs"

# 1) commands.rs: log after update
p = os.path.join(R, r"crates\liquid-verre\src\runtime\commands.rs")
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = """            app.content_store.update(
                &app.gfx.device,
                &app.gfx.ctx,
                id,
                w_phys,
                h_phys,
                &buffer,
                dirty,
            );
            true"""
new = """            app.content_store.update(
                &app.gfx.device,
                &app.gfx.ctx,
                id,
                w_phys,
                h_phys,
                &buffer,
                dirty,
            );
            log::info!(
                target: "verify",
                "DIAG cmd UpdateContent id={id} {w_phys}x{h_phys} len={} layers={}",
                buffer.len(),
                app.content_store.diag_len()
            );
            true"""
assert old in c
c = c.replace(old, new, 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)

# 2) content_layer: diag_len
p = os.path.join(R, r"crates\liquid-verre\src\gfx\content_layer.rs")
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = "    /// 渲染侧查表（draw_blocks 交错绘制用）。"
new = """    #[cfg(debug_assertions)]
    pub fn diag_len(&self) -> usize {
        self.layers.len()
    }

    /// 渲染侧查表（draw_blocks 交错绘制用）。"""
# actually we need it in release too (diagnostics build)
new = """    /// TEMP DIAG (remove after content-layer bring-up): store size.
    pub fn diag_len(&self) -> usize {
        self.layers.len()
    }

    /// 渲染侧查表（draw_blocks 交错绘制用）。"""
assert old in c
c = c.replace(old, new, 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)

# 3) glass_renderer: log in draw_content_for_block (first hit only via Cell)
p = os.path.join(R, r"crates\liquid-verre\src\gfx\glass_renderer.rs")
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = "        set4(&mut cb, 2, canvas_w, canvas_h, 0.0, 0.0);\n        if update_cbuffer(ctx, &self.ccb, &cb).is_err() {\n            return;\n        }"
new = """        set4(&mut cb, 2, canvas_w, canvas_h, 0.0, 0.0);
        if !self.diag_logged.get() {
            self.diag_logged.set(true);
            log::info!(
                target: "verify",
                "DIAG draw_content id={} rect=({},{},{},{}) tex={}x{} radius={:.1}",
                block.id, rx0, ry0, rw, rh, layer.w, layer.h, radius
            );
        }
        if update_cbuffer(ctx, &self.ccb, &cb).is_err() {
            return;
        }"""
assert old in c
c = c.replace(old, new, 1)
old = "    /// 本帧真正执行了 Draw 的块数（metrics 用）\n    drawn: std::cell::Cell<usize>,"
new = "    /// 本帧真正执行了 Draw 的块数（metrics 用）\n    drawn: std::cell::Cell<usize>,\n    /// TEMP DIAG: one-shot flag for draw_content log\n    diag_logged: std::cell::Cell<bool>,"
assert old in c
c = c.replace(old, new, 1)
old = "            drawn: std::cell::Cell::new(0),\n        })"
new = "            drawn: std::cell::Cell::new(0),\n            diag_logged: std::cell::Cell::new(false),\n        })"
assert old in c
c = c.replace(old, new, 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("diagnostics added")
