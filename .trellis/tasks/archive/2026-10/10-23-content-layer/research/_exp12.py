import io, os
R = r"D:\github-project\liquid-verre-rs"
p = os.path.join(R, r"crates\liquid-verre\src\gfx\glass_renderer.rs")
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()

# re-add readback helper (fixed version)
idx = c.find("\nconst CONTENT_CB_FLOATS")
assert idx > 0
helper = '''

/// EXPERIMENT: copy 1 px of the bound RT to a staging texture and read it.
fn readback_probe(
    ctx: &ID3D11DeviceContext,
    device: &ID3D11Device,
    x: i32,
    y: i32,
) -> String {
    use windows::core::Interface;
    use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
    unsafe {
        let mut rts: [Option<ID3D11RenderTargetView>; 1] = [None];
        let mut ds: Option<ID3D11DepthStencilView> = None;
        ctx.OMGetRenderTargets(Some(&mut rts), Some(&mut ds));
        let Some(rt) = rts[0].clone() else {
            return "NO_RT_BOUND".to_string();
        };
        let tex: ID3D11Texture2D = match rt.cast() {
            Ok(t) => t,
            Err(_) => return "RT_NOT_TEXTURE".to_string(),
        };
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
        format!("desc={}x{} fmt={:?} px={:?}", desc.Width, desc.Height, desc.Format.0, val)
    }
}
'''
c = c[:idx] + helper + c[idx:]

# call it inside draw_content_for_block after Draw, gated by READY_SEEN
old = """            ctx.Draw(6, 0);
            static ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
            if ONCE.swap(false, std::sync::atomic::Ordering::Relaxed) {
                log::info!(
                    target: "verify",
                    "DIAG5 content Draw executed id={} scissor=({},{},{},{})",
                    block.id,
                    sx,
                    sy,
                    cx1,
                    cy1
                );
            }"""
new = """            ctx.Draw(6, 0);
            static ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
            if READY_SEEN.load(std::sync::atomic::Ordering::Relaxed)
                && ONCE.swap(false, std::sync::atomic::Ordering::Relaxed)
            {
                log::info!(
                    target: "verify",
                    "DIAG6 post-ready readback: {}",
                    readback_probe(ctx, device_for_readback, rx0 + 200, ry0 + 200)
                );
            }"""
assert old in c, "DIAG5 block not found"
c = c.replace(old, new, 1)

# READY_SEEN static + device param plumbing
idx = c.find("/// EXPERIMENT: copy 1 px")
c = c[:idx] + "\npub(crate) static READY_SEEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);\n" + c[idx:]

old = "    fn draw_content_for_block(\n        &self,\n        ctx: &ID3D11DeviceContext,\n        block: &DrawBlock,"
new = "    fn draw_content_for_block(\n        &self,\n        ctx: &ID3D11DeviceContext,\n        device_for_readback: &ID3D11Device,\n        block: &DrawBlock,"
assert old in c
c = c.replace(old, new, 1)
old = """                    self.draw_content_for_block(
                        ctx,
                        block,"""
new = """                    self.draw_content_for_block(
                        ctx,
                        device,
                        block,"""
assert old in c
c = c.replace(old, new, 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)

# compositor: set READY_SEEN when shot frame fires AFTER a steady_ready marker is impossible here;
# simpler: playground writes ready -> compositor can't see it. Use env again: script sets
# GLASS_SHOT_AT_MS=2800 and playground heartbeat keeps frames; mark READY_SEEN from a tiny
# env read at construction is wrong timing. Instead: mark it when shot_at already fired once
# (second shot opportunity) -- no. Use: playground logs steady_ready; lib can't see logs.
# Practical: set READY_SEEN from a repeated internal shot cadence: script uses
# GLASS_SHOT_EVERY_MS=600 -> every rendered frame after 600ms is a shot; readback once at >=3s.
# Simplest for the experiment: set READY_SEEN from env GLASS_DIAG_READBACK=1 checked per-frame cheaply?
# -> gate it once at construction (env read at construction is allowed for diag):
old = "pub(crate) static READY_SEEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);"
new = "pub(crate) static READY_SEEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);\n"
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)

# app.rs: set READY_SEEN when a render happens with time > 2.5s since t0 (frame-time gate, no env)
ap = os.path.join(R, r"crates\liquid-verre\src\runtime\app.rs")
with io.open(ap, "r", encoding="utf-8") as f:
    a = f.read()
old = "pub(crate) fn render_monitor(app: &mut App, mi: usize, ctx: &FrameCtx, full_redraw: bool) -> bool {"
new = """pub(crate) fn render_monitor(app: &mut App, mi: usize, ctx: &FrameCtx, full_redraw: bool) -> bool {
    // EXPERIMENT: arm the one-shot readback once the app has been up > 2.5 s.
    if !crate::gfx::glass_renderer::READY_SEEN.load(std::sync::atomic::Ordering::Relaxed)
        && app.t0.elapsed().as_millis() > 2500
    {
        crate::gfx::glass_renderer::READY_SEEN.store(true, std::sync::atomic::Ordering::Relaxed);
    }"""
assert old in a
a = a.replace(old, new, 1)
with io.open(ap, "w", encoding="utf-8", newline="\n") as f:
    f.write(a)
print("readback v2 armed by t0>2.5s")
