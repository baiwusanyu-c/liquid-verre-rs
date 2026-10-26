import io, os
p = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\glass_renderer.rs"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()

old = """        unsafe {
            ctx.PSSetShaderResources(9, Some(&[Some(layer.srv.clone())]));
            ctx.RSSetScissorRects(Some(&[WinRect {
                left: sx,
                top: sy,
                right: ex,
                bottom: ey,
            }]));
            ctx.VSSetShader(&self.cvs, None);
            ctx.PSSetShader(&self.cps, None);
            ctx.Draw(6, 0);
            // 切回玻璃管线：下一块玻璃的 cbuffer(t0..t8/s0/b0) 绑定原样在位，
            // 只需要恢复两个 shader 指针。
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShader(&self.ps, None);
        }"""
new = """        unsafe {
            ctx.PSSetShaderResources(9, Some(&[Some(layer.srv.clone())]));
            // 内容 pass 的 viewport 必须是块矩形本身：content_vs 的 NDC 以
            // c0 矩形为 [0,1] 域，若沿用玻璃 pass 的全画布 viewport，块内容会被
            // 拉伸到全屏（2026-10-26 bring-up 实测：采样窗口读到放大 2.6x 的
            // backdrop 纹理，误判成「内容未渲染」）。scissor 再按画布边界收窄。
            ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: rx0 as f32,
                TopLeftY: ry0 as f32,
                Width: rw as f32,
                Height: rh as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            let cx1 = ex.min(out_w);
            let cy1 = ey.min(out_h);
            ctx.RSSetScissorRects(Some(&[WinRect {
                left: sx,
                top: sy,
                right: cx1,
                bottom: cy1,
            }]));
            ctx.VSSetShader(&self.cvs, None);
            ctx.PSSetShader(&self.cps, None);
            ctx.Draw(6, 0);
            // 切回玻璃管线：下一块玻璃的 cbuffer(t0..t8/s0/b0) 绑定原样在位，
            // 但 viewport 是全画布的，必须一起恢复。
            ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: out_w as f32,
                Height: out_h as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShader(&self.ps, None);
        }"""
assert old in c, "draw_content_for_block unsafe block not found"
c = c.replace(old, new, 1)

# helper needs out_w/out_h for scissor clamp + viewport restore
old = "    fn draw_content_for_block(\n        &self,\n        ctx: &ID3D11DeviceContext,\n        block: &DrawBlock,\n        radius: f32,\n        layer: &crate::gfx::content_layer::ContentLayer,\n        canvas_w: f32,\n        canvas_h: f32,\n    ) {"
new = "    fn draw_content_for_block(\n        &self,\n        ctx: &ID3D11DeviceContext,\n        block: &DrawBlock,\n        radius: f32,\n        layer: &crate::gfx::content_layer::ContentLayer,\n        canvas_w: f32,\n        canvas_h: f32,\n        out_w: i32,\n        out_h: i32,\n    ) {"
assert old in c, "helper sig not found"
c = c.replace(old, new, 1)

old = """                    self.draw_content_for_block(
                        ctx,
                        block,
                        p.corner_radius,
                        layer,
                        canvas_w,
                        canvas_h,
                    );"""
new = """                    self.draw_content_for_block(
                        ctx,
                        block,
                        p.corner_radius,
                        layer,
                        canvas_w,
                        canvas_h,
                        out_w,
                        out_h,
                    );"""
assert old in c, "call site not found"
c = c.replace(old, new, 1)

with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("viewport fix applied")
