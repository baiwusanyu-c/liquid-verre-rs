import io, os
p = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\glass_renderer.rs"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = """            ctx.Draw(6, 0);
            // EXPERIMENT v3: also draw with the known-good border shaders
            ctx.VSSetShader(&self.bvs, None);
            ctx.PSSetShader(&self.bps, None);
            ctx.Draw(6, 0);"""
new = """            ctx.Draw(6, 0);
            // EXPERIMENT v4: flush + read back one pixel of the RT to see if
            // the content draw reached the GPU at all.
            ctx.Flush();
            static RB: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
            if RB.swap(false, std::sync::atomic::Ordering::Relaxed) {
                let hr = readback_probe(ctx, device_for_readback, rx0 + 100, ry0 + 100);
                log::info!(target: "verify", "DIAG4 readback@(rx+100,ry+100) -> {}", hr);
            }"""
assert old in c
c = c.replace(old, new, 1)

# helper needs device; add param
old2 = "    fn draw_content_for_block(\n        &self,\n        ctx: &ID3D11DeviceContext,\n        block: &DrawBlock,"
new2 = "    fn draw_content_for_block(\n        &self,\n        ctx: &ID3D11DeviceContext,\n        device_for_readback: &ID3D11Device,\n        block: &DrawBlock,"
assert old2 in c
c = c.replace(old2, new2, 1)
old3 = """                    self.draw_content_for_block(
                        ctx,
                        block,"""
new3 = """                    self.draw_content_for_block(
                        ctx,
                        device,
                        block,"""
assert old3 in c
c = c.replace(old3, new3, 1)

# add readback helper at module level
idx = c.find("\nstatic DIAG3")
assert idx > 0
helper = '''
/// EXPERIMENT: copy 1 px of the bound RT to a staging texture and read it.
fn readback_probe(
    ctx: &ID3D11DeviceContext,
    device: &ID3D11Device,
    x: i32,
    y: i32,
) -> String {
    unsafe {
        let mut rt = None;
        let mut ds = None;
        ctx.OMGetRenderTargets(0, Some(&mut rt), Some(&mut ds));
        let Some(rt) = rt else {
            return "NO_RT_BOUND".to_string();
        };
        let mut tex = None;
        if rt.QueryInterface(&mut tex).is_err() {
            return "RT_NOT_TEXTURE".to_string();
        }
        let tex: windows::Win32::Graphics::Direct3D11::ID3D11Texture2D = tex;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        tex.GetDesc(&mut desc);
        let sd = D3D11_TEXTURE2D_DESC {
            Width: 1,
            Height: 1,
            MipLevels: 1,
            ArraySize: 1,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
            ..desc
        };
        let mut stage: Option<ID3D11Texture2D> = None;
        if let Err(e) = device.CreateTexture2D(&sd, None, Some(&mut stage)) {
            return format!("CREATE_STAGING {e}");
        }
        let stage = stage.unwrap();
        let src_box = D3D11_BOX {
            left: x.max(0) as u32,
            top: y.max(0) as u32,
            front: 0,
            right: (x.max(0) + 1) as u32,
            bottom: (y.max(0) + 1) as u32,
            back: 1,
        };
        ctx.CopySubresourceRegion(&stage, 0, 0, 0, 0, &tex, 0, Some(&src_box));
        ctx.Flush();
        let mut m = D3D11_MAPPED_SUBRESOURCE::default();
        if let Err(e) = ctx.Map(&stage, 0, D3D11_MAP_READ, 0, Some(&mut m)) {
            return format!("MAP {e}");
        }
        let p = m.pData as *const u8;
        let val = (*p, *p.add(1), *p.add(2), *p.add(3));
        ctx.Unmap(&stage, 0);
        format!("desc={}x{} fmt={:?} px={:?}", desc.Width, desc.Height, desc.Format, val)
    }
}
'''
c = c[:idx] + helper + c[idx:]
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("exp v4 readback added")
