import io, os
R = r"D:\github-project\liquid-verre-rs"

# restore real shaders
for name in ("content_vs.hlsl", "content_ps.hlsl"):
    src = os.path.join(R, r"crates\liquid-verre\shaders", name)
    keep = src + ".keep"
    io.open(src, "w", encoding="utf-8", newline="\n").write(io.open(keep, encoding="utf-8").read())
    os.unlink(keep)
print("shaders restored")

p = os.path.join(R, r"crates\liquid-verre\src\gfx\glass_renderer.rs")
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()

# 1) hoist srvs out of the prologue unsafe block so the loop can re-bind
old = """            let srvs = [
                Some(capture.clone()),
                Some(frost.srv.clone()),
                Some(soft.srv.clone()),
                Some(env16.srv.clone()),
                Some(self.lut_srv.clone()),
                Some(blur0.srv.clone()),
                Some(blur1.srv.clone()),
                Some(blur2.srv.clone()),
                Some(blur3.srv.clone()),
            ];
            ctx.PSSetShaderResources(0, Some(&srvs));
        }"""
new = """            ctx.PSSetShaderResources(0, Some(&srvs));
        }"""
assert old in c, "prologue srvs not found"
c = c.replace(old, new, 1)

old = """            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShader(&self.ps, None);
            ctx.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
            ctx.VSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
            ctx.PSSetShaderResources(0, Some(&srvs));"""
# hmm - the PSSetShaderResources(0, srvs) line placement: it was after the CB binds. Re-check structure:
if old not in c:
    # find actual
    i = c.find("let srvs = [")
    print("CTX:", c[i-700:i+400])
    raise SystemExit("anchor mismatch, printed context")
c = c.replace(old, old, 1)  # no-op, we only removed the array definition above

# define srvs before the prologue unsafe block instead
old2 = """        unsafe {
            ctx.IASetInputLayout(None);
            ctx.IASetPrimitiveTopology(TOPO_TRIANGLE);
            ctx.OMSetRenderTargets(Some(&[Some(out_rtv.clone())]), None);"""
new2 = """        let srvs = [
            Some(capture.clone()),
            Some(frost.srv.clone()),
            Some(soft.srv.clone()),
            Some(env16.srv.clone()),
            Some(self.lut_srv.clone()),
            Some(blur0.srv.clone()),
            Some(blur1.srv.clone()),
            Some(blur2.srv.clone()),
            Some(blur3.srv.clone()),
        ];
        unsafe {
            ctx.IASetInputLayout(None);
            ctx.IASetPrimitiveTopology(TOPO_TRIANGLE);
            ctx.OMSetRenderTargets(Some(&[Some(out_rtv.clone())]), None);"""
assert old2 in c, "prologue start not found"
c = c.replace(old2, new2, 1)
print("srvs hoisted")

# 2) helper: take srvs, re-bind glass pipeline fully after content draw
old3 = "    fn draw_content_for_block(\n        &self,\n        ctx: &ID3D11DeviceContext,\n        block: &DrawBlock,\n        radius: f32,\n        layer: &crate::gfx::content_layer::ContentLayer,\n        canvas_w: f32,\n        canvas_h: f32,\n        out_w: i32,\n        out_h: i32,\n    ) {"
new3 = "    fn draw_content_for_block(\n        &self,\n        ctx: &ID3D11DeviceContext,\n        block: &DrawBlock,\n        radius: f32,\n        layer: &crate::gfx::content_layer::ContentLayer,\n        canvas_w: f32,\n        canvas_h: f32,\n        out_w: i32,\n        out_h: i32,\n        glass_srvs: &[Option<ID3D11ShaderResourceView>; 9],\n    ) {"
assert old3 in c, "helper sig not found"
c = c.replace(old3, new3, 1)

old4 = """            ctx.Draw(6, 0);
            log::info!(
                target: "verify",
                "DIAG Draw(6) issued id={} vs={} ps={} scissor=({},{},{},{})",
                block.id,
                &self.cvs as *const _ as usize,
                &self.cps as *const _ as usize,
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
new4 = """            ctx.Draw(6, 0);
            // 切回玻璃管线。t9 的 content SRV 绝不能活到下一块玻璃：bind_lut 在
            // 逐块 LUT 切换时只重绑 t4 单槽，若 content pass 之后不整体重绑
            // t0..t8，玻璃采样与绘制会踩 gotchas #3 的静默丢弃（2026-10-26
            // bring-up 实测：content pass 后下一块玻璃整块消失，红屏实验定位）。
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShader(&self.ps, None);
            ctx.PSSetShaderResources(0, Some(glass_srvs));
            let null9: [Option<ID3D11ShaderResourceView>; 1] = [None];
            ctx.PSSetShaderResources(9, Some(&null9));
        }
    }"""
assert old4 in c, "helper tail not found"
c = c.replace(old4, new4, 1)

# 3) call site: pass srvs
old5 = """                    self.draw_content_for_block(
                        ctx,
                        block,
                        p.corner_radius,
                        layer,
                        canvas_w,
                        canvas_h,
                        out_w,
                        out_h,
                    );"""
new5 = """                    self.draw_content_for_block(
                        ctx,
                        block,
                        p.corner_radius,
                        layer,
                        canvas_w,
                        canvas_h,
                        out_w,
                        out_h,
                        &srvs,
                    );"""
assert old5 in c, "call site not found"
c = c.replace(old5, new5, 1)

# 4) remove the diag draw_content one-shot log (keep cmd-side diag for now)
old6 = """        if !self.diag_logged.get() {
            self.diag_logged.set(true);
            log::info!(
                target: "verify",
                "DIAG draw_content id={} rect=({},{},{},{}) tex={}x{} radius={:.1}",
                block.id, rx0, ry0, rw, rh, layer.w, layer.h, radius
            );
        }
"""
assert old6 in c
c = c.replace(old6, "", 1)

with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("hazard fix applied")
