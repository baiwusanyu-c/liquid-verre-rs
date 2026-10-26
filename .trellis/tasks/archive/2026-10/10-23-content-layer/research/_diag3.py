import io, os
p = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\glass_renderer.rs"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = """        if update_cbuffer(ctx, &self.ccb, &cb).is_err() {
            return;
        }
        unsafe {
            ctx.PSSetShaderResources(9, Some(&[Some(layer.srv.clone())]));"""
new = """        if let Err(e) = update_cbuffer(ctx, &self.ccb, &cb) {
            log::info!(target: "verify", "DIAG ccb update failed: {e}");
            return;
        }
        unsafe {
            ctx.PSSetShaderResources(9, Some(&[Some(layer.srv.clone())]));"""
assert old in c
c = c.replace(old, new, 1)
old2 = """            ctx.VSSetShader(&self.cvs, None);
            ctx.PSSetShader(&self.cps, None);
            ctx.Draw(6, 0);
            // 切回玻璃管线：viewport 全程未动（全画布），只需恢复 shader 指针。
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShader(&self.ps, None);
        }
    }"""
new2 = """            ctx.VSSetShader(&self.cvs, None);
            ctx.PSSetShader(&self.cps, None);
            ctx.Draw(6, 0);
            log::info!(
                target: "verify",
                "DIAG Draw(6) issued id={} vs={} ps={} scissor=({},{},{},{})",
                block.id,
                self.cvs as *const _ as usize,
                self.cps as *const _ as usize,
                sx,
                sy,
                cx1,
                cy1
            );
            // 切回玻璃管线：viewport 全程未动（全画布），只需恢复 shader 指针。
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShader(&self.ps, None);
        }
    }"""
assert old2 in c
c = c.replace(old2, new2, 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("diag v2 added")
