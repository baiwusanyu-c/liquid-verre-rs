import io, os
p = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\glass_renderer.rs"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()

# 1) remove the readback experiment block
old = """            ctx.Draw(6, 0);
            // EXPERIMENT v4: flush + read back one pixel of the RT to see if
            // the content draw reached the GPU at all.
            ctx.Flush();
            static RB: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
            if RB.swap(false, std::sync::atomic::Ordering::Relaxed) {
                let hr = readback_probe(ctx, device_for_readback, rx0 + 100, ry0 + 100);
                log::info!(target: "verify", "DIAG4 readback@(rx+100,ry+100) -> {}", hr);
            }"""
new = "            ctx.Draw(6, 0);"
assert old in c
c = c.replace(old, new, 1)

# 2) remove readback helper entirely
i0 = c.find("/// EXPERIMENT: copy 1 px of the bound RT")
i1 = c.find("\nstatic DIAG3")
assert 0 < i0 < i1
c = c[:i0] + c[i1:]

# 3) remove DIAG2 block
i0 = c.find("        {\n            static CALLS:")
i1 = c.find("        }\n        if store.is_some() {")
assert i0 > 0 and i1 > i0
c = c[:i0] + "        " + c[i1+len("        }"):]

# 4) remove DIAG3 static + hit log
c = c.replace("\nstatic DIAG3: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);\n", "")
old = """            if let Some(store) = store {
                if store.get(block.id).is_some() && DIAG3.swap(false, std::sync::atomic::Ordering::Relaxed) {
                    log::info!(target: "verify", "DIAG3 first content hit id={}", block.id);
                }
                if let Some(layer) = store.get(block.id) {"""
new = """            if let Some(store) = store {
                if let Some(layer) = store.get(block.id) {"""
assert old in c
c = c.replace(old, new, 1)

# 5) helper: drop device param, restore proper scissor set (was left as None by exp5)
old = "    fn draw_content_for_block(\n        &self,\n        ctx: &ID3D11DeviceContext,\n        device_for_readback: &ID3D11Device,\n        block: &DrawBlock,"
new = "    fn draw_content_for_block(\n        &self,\n        ctx: &ID3D11DeviceContext,\n        block: &DrawBlock,"
assert old in c
c = c.replace(old, new, 1)
old = """                    self.draw_content_for_block(
                        ctx,
                        device,
                        block,"""
new = """                    self.draw_content_for_block(
                        ctx,
                        block,"""
assert old in c
c = c.replace(old, new, 1)

# find the scissor line currently in helper (exp5 left None)
i = c.find("ctx.RSSetScissorRects(None); // EXPERIMENT: no scissor")
assert i > 0, "exp5 scissor line not found"
c = c[:i] + """ctx.RSSetScissorRects(Some(&[WinRect {
                left: sx,
                top: sy,
                right: cx1,
                bottom: cy1,
            }]));""" + c[i+len("ctx.RSSetScissorRects(None); // EXPERIMENT: no scissor"):]

# 6) remove border control-draw experiment (exp3)
old = """            ctx.Draw(6, 0);
            // EXPERIMENT v3: also draw with the known-good border shaders
            ctx.VSSetShader(&self.bvs, None);
            ctx.PSSetShader(&self.bps, None);
            ctx.Draw(6, 0);"""
new = "            ctx.Draw(6, 0);"
if old in c:
    c = c.replace(old, new, 1)
    print("exp3 border control removed")

with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("clean rebuild of helper done")
