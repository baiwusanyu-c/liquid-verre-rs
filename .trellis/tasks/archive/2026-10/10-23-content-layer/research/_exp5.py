import io, os
R = r"D:\github-project\liquid-verre-rs"
vs = os.path.join(R, r"crates\liquid-verre\shaders\content_vs.hlsl")
ps = os.path.join(R, r"crates\liquid-verre\shaders\content_ps.hlsl")
if not os.path.exists(vs + ".keep"):
    io.open(vs + ".keep", "w", encoding="utf-8", newline="\n").write(io.open(vs, encoding="utf-8").read())
    io.open(ps + ".keep", "w", encoding="utf-8", newline="\n").write(io.open(ps, encoding="utf-8").read())
vs_exp = """struct VSOut { float4 pos : SV_Position; };
VSOut vs_main(uint vid : SV_VertexID)
{
    static const float2 q[6] = {
        float2(-1,-1), float2(1,-1), float2(-1,1),
        float2(-1,1), float2(1,-1), float2(1,1)
    };
    VSOut o; o.pos = float4(q[vid], 0.0, 1.0); return o;
}
"""
ps_exp = """float4 ps_main(float4 vpos : SV_Position) : SV_Target
{
    return float4(0.0, 0.0, 1.0, 1.0);
}
"""
io.open(vs, "w", encoding="utf-8", newline="\n").write(vs_exp)
io.open(ps, "w", encoding="utf-8", newline="\n").write(ps_exp)

gp = os.path.join(R, r"crates\liquid-verre\src\gfx\glass_renderer.rs")
with io.open(gp, "r", encoding="utf-8") as f:
    c = f.read()
old = """            ctx.RSSetScissorRects(Some(&[WinRect {
                left: sx,
                top: sy,
                right: cx1,
                bottom: cy1,
            }]));
            ctx.VSSetShader(&self.cvs, None);"""
new = """            ctx.RSSetScissorRects(None); // EXPERIMENT: no scissor
            ctx.VSSetShader(&self.cvs, None);"""
assert old in c
c = c.replace(old, new, 1)
with io.open(gp, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("exp v2: red fullscreen, no scissor")
