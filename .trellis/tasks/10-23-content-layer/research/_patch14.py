import io

path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\glass_renderer.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()

# Add draw_contents after draw_borders (which ends with "    }\n}")
# Find the end of draw_borders
old_end = """        unsafe {
            ctx.RSSetScissorRects(None);
            ctx.OMSetRenderTargets(None, None);
        }
    }
}"""

new_end = """        unsafe {
            ctx.RSSetScissorRects(None);
            ctx.OMSetRenderTargets(None, None);
        }
    }

    /// Draw content layer overlays on top of glass, below borders.
    ///
    /// Each block with an active content texture gets one `Draw(6)` call.
    /// Blocks without content are silently skipped (zero-cost path, FR5).
    /// Content is clipped to the block''s round-rect shape using the same
    /// `SdRoundBox` formula as the glass and border passes (C5: single source).
    pub(crate) fn draw_contents(
        &self,
        ctx: &ID3D11DeviceContext,
        out: &BorderTarget<'_>,
        blocks: &[DrawBlock],
        order: &[usize],
        store: &crate::gfx::content_layer::ContentStore,
    ) {
        let (out_rtv, out_w, out_h, canvas_w, canvas_h) =
            (out.rtv, out.w, out.h, out.canvas[0], out.canvas[1]);
        unsafe {
            ctx.IASetInputLayout(None);
            ctx.IASetPrimitiveTopology(TOPO_TRIANGLE);
            ctx.OMSetRenderTargets(Some(&[Some(out_rtv.clone())]), None);
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
            ctx.PSSetSamplers(0, Some(&[Some(self.csampler.clone())]));
            ctx.VSSetConstantBuffers(0, Some(&[Some(self.ccb.clone())]));
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.ccb.clone())]));
        }

        for &index in order {
            let Some(block) = blocks.get(index) else {
                continue;
            };
            let Some(layer) = store.get(block.id) else {
                continue;
            };

            // Block draw rectangle (canvas-local physical pixels).
            let [rx0, ry0, rx1, ry1] = block.draw_rect();
            let sx = rx0.max(0).min(out_w);
            let sy = ry0.max(0).min(out_h);
            let ex = rx1.max(0).min(out_w);
            let ey = ry1.max(0).min(out_h);
            if ex <= sx || ey <= sy {
                continue;
            }

            // Bind content texture SRV to t0.
            unsafe {
                ctx.PSSetShaderResources(0, Some(&[Some(layer.srv.clone())]));
            }

            // ContentCb: c0 = block rect, c1 = tex size + radius + AA, c2 = canvas size
            let bw = (rx1 - rx0) as f32;
            let bh = (ry1 - ry0) as f32;
            let mut cb = ContentCb { v: [0.0; CONTENT_CB_FLOATS] };
            let set4 = |cb: &mut ContentCb, slot: usize, a: f32, b: f32, c: f32, d: f32| {
                let i = slot * 4;
                cb.v[i] = a;
                cb.v[i + 1] = b;
                cb.v[i + 2] = c;
                cb.v[i + 3] = d;
            };
            set4(&mut cb, 0, rx0 as f32, ry0 as f32, bw, bh);
            set4(&mut cb, 1, layer.w as f32, layer.h as f32, block.radius, 1.25);
            set4(&mut cb, 2, canvas_w, canvas_h, 0.0, 0.0);

            if update_cbuffer(ctx, &self.ccb, &cb).is_err() {
                continue;
            }

            unsafe {
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
            let null: [Option<ID3D11ShaderResourceView>; 1] = [None];
            ctx.PSSetShaderResources(0, Some(&null));
            ctx.RSSetScissorRects(None);
            ctx.OMSetRenderTargets(None, None);
        }
    }
}"""

assert old_end in content, "old_end not found"
content = content.replace(old_end, new_end, 1)
print("draw_contents method added")

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("Written")
