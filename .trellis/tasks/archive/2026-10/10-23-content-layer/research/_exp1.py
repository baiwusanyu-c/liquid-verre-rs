import io, os
R = r"D:\github-project\liquid-verre-rs"

# revert viewport change (back to full-canvas viewport; VS already maps block rect via NDC)
p = os.path.join(R, r"crates\liquid-verre\src\gfx\glass_renderer.rs")
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = """            // 内容 pass 的 viewport 必须是块矩形本身：content_vs 的 NDC 以
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
            }]));"""
new = """            let cx1 = ex.min(out_w);
            let cy1 = ey.min(out_h);
            ctx.RSSetScissorRects(Some(&[WinRect {
                left: sx,
                top: sy,
                right: cx1,
                bottom: cy1,
            }]));"""
assert old in c
c = c.replace(old, new, 1)
old2 = """            ctx.Draw(6, 0);
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
            ctx.VSSetShader(&self.vs, None);"""
new2 = """            ctx.Draw(6, 0);
            // 切回玻璃管线：viewport 全程未动（全画布），只需恢复 shader 指针。
            ctx.VSSetShader(&self.vs, None);"""
assert old2 in c
c = c.replace(old2, new2, 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("viewport reverted")

# EXPERIMENT: content_ps hard red, no texture dependency
ps = os.path.join(R, r"crates\liquid-verre\shaders\content_ps.hlsl")
with io.open(ps, "r", encoding="utf-8") as f:
    src = f.read()
with io.open(ps + ".orig", "w", encoding="utf-8", newline="\n") as f:
    f.write(src)
red = src.replace(
    "    float4 tex = texContent.Load(int3(t, 0));",
    "    float4 tex = float4(0.0, 0.0, 1.0, 1.0); // EXPERIMENT: ignore texture"
)
with io.open(ps, "w", encoding="utf-8", newline="\n") as f:
    f.write(red)
print("PS experiment: hard red")
