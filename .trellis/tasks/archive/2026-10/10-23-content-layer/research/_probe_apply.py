import io, shutil, os

R = r"D:\github-project\liquid-verre-rs"
research = os.path.join(R, ".trellis", "tasks", "10-23-content-layer", "research")

gr = os.path.join(R, r"crates\liquid-verre\src\gfx\glass_renderer.rs")
cp = os.path.join(R, r"crates\liquid-verre\src\gfx\compositor.rs")
exe = os.path.join(R, r"target\release\liquid-verre-playground.exe")

# backups
shutil.copy2(gr, os.path.join(research, "glass_renderer.interleaved.bak"))
shutil.copy2(cp, os.path.join(research, "compositor.interleaved.bak"))
shutil.copy2(exe, os.path.join(research, "vc_interleaved.exe"))
print("backups + interleaved exe archived")

# ---- patch glass_renderer: append batch pass method before final impl close ----
with io.open(gr, "r", encoding="utf-8") as f:
    g = f.read()

anchor = g.rfind("\n}\n")
assert anchor > 0, "impl close not found"
batch_method = '''
    /// PROBE-ONLY (2026-10-26 discrimination self-check): batch content pass,
    /// drawn after ALL glass blocks. Structurally wrong per FR3 -- lower-block
    /// content floats above upper-block glass. Do not ship.
    #[allow(dead_code)]
    pub(crate) fn draw_contents_batch(
        &self,
        ctx: &ID3D11DeviceContext,
        rtv: &ID3D11RenderTargetView,
        out_w: i32,
        out_h: i32,
        blocks: &[DrawBlock],
        order: &[usize],
        store: &crate::gfx::content_layer::ContentStore,
        canvas_w: f32,
        canvas_h: f32,
    ) {
        if store.is_empty() {
            return;
        }
        unsafe {
            ctx.IASetInputLayout(None);
            ctx.IASetPrimitiveTopology(TOPO_TRIANGLE);
            ctx.OMSetRenderTargets(Some(&[Some(rtv.clone())]), None);
            ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: out_w as f32,
                Height: out_h as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            ctx.RSSetState(&self.raster);
            ctx.OMSetBlendState(&self.blend, Some(&[0.0; 4]), 0xffff_ffff);
            ctx.OMSetDepthStencilState(&self.depth, 0);
            ctx.VSSetShader(&self.cvs, None);
            ctx.PSSetShader(&self.cps, None);
            ctx.VSSetConstantBuffers(1, Some(&[Some(self.ccb.clone())]));
            ctx.PSSetConstantBuffers(1, Some(&[Some(self.ccb.clone())]));
        }
        for &index in order {
            let Some(block) = blocks.get(index) else {
                continue;
            };
            let Some(layer) = store.get(block.id) else {
                continue;
            };
            let rx0 = block.x.round() as i32;
            let ry0 = block.y.round() as i32;
            let rw = (block.width.round() as i32).max(1);
            let rh = (block.height.round() as i32).max(1);
            let sx = rx0.max(0);
            let sy = ry0.max(0);
            let ex = rx0.saturating_add(rw).max(0);
            let ey = ry0.saturating_add(rh).max(0);
            let mut cb = ContentCb {
                v: [0.0; CONTENT_CB_FLOATS],
            };
            let set4 = |cb: &mut ContentCb, slot: usize, a: f32, b: f32, c: f32, d: f32| {
                let i = slot * 4;
                cb.v[i] = a;
                cb.v[i + 1] = b;
                cb.v[i + 2] = c;
                cb.v[i + 3] = d;
            };
            set4(&mut cb, 0, rx0 as f32, ry0 as f32, rw as f32, rh as f32);
            set4(&mut cb, 1, layer.w as f32, layer.h as f32, block.radius, 1.25);
            set4(&mut cb, 2, canvas_w, canvas_h, 0.0, 0.0);
            if update_cbuffer(ctx, &self.ccb, &cb).is_err() {
                continue;
            }
            unsafe {
                ctx.PSSetShaderResources(9, Some(&[Some(layer.srv.clone())]));
                ctx.RSSetScissorRects(Some(&[WinRect {
                    left: sx,
                    top: sy,
                    right: ex,
                    bottom: ey,
                }]));
                ctx.Draw(6, 0);
            }
        }
        unsafe {
            let null9: [Option<ID3D11ShaderResourceView>; 1] = [None];
            ctx.PSSetShaderResources(9, Some(&null9));
            ctx.RSSetScissorRects(None);
            ctx.OMSetRenderTargets(None, None);
        }
    }
'''
# insert before the last "\n}\n" (impl block close)
g = g[:anchor] + batch_method + "}\n" + g[anchor+3:]
with io.open(gr, "w", encoding="utf-8", newline="\n") as f:
    f.write(g)
print("probe: batch method appended")

# ---- patch compositor: draw_blocks gets None + batch call before borders ----
with io.open(cp, "r", encoding="utf-8") as f:
    k = f.read()
old = "                hover_id,\n                Some(content_store),\n            );"
new = """                hover_id,
                None,
            );
            glass.draw_contents_batch(
                &gfx.ctx,
                &self.rt.rtv,
                self.rt.w,
                self.rt.h,
                blocks,
                order,
                content_store,
                self.width as f32,
                self.height as f32,
            );"""
assert old in k, "compositor call not found"
k = k.replace(old, new, 1)
with io.open(cp, "w", encoding="utf-8", newline="\n") as f:
    f.write(k)
print("probe: compositor switched to batch")
