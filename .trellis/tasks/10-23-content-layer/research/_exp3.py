import io, os
R = r"D:\github-project\liquid-verre-rs"
vs = os.path.join(R, r"crates\liquid-verre\shaders\content_vs.hlsl")
ps = os.path.join(R, r"crates\liquid-verre\shaders\content_ps.hlsl")
for src in (vs, ps):
    keep = src + ".keep"
    if not os.path.exists(keep):
        io.open(keep, "w", encoding="utf-8", newline="\n").write(io.open(src, encoding="utf-8").read())
vs_exp = """// EXPERIMENT: fullscreen quad, zero CB dependency
struct VSOut { float4 pos : SV_Position; };
VSOut vs_main(uint vid : SV_VertexID)
{
    static const float2 q[6] = {
        float2(-1,-1), float2(1,-1), float2(-1,1),
        float2(-1,1), float2(1,-1), float2(1,1)
    };
    VSOut o; o.pos = float4(q[vid], 0.0, 1.0); return o;
}
"""
ps_exp = """// EXPERIMENT: constant red, no CB / no texture
float4 ps_main(float4 vpos : SV_Position) : SV_Target
{
    return float4(0.0, 0.0, 1.0, 1.0);
}
"""
io.open(vs, "w", encoding="utf-8", newline="\n").write(vs_exp)
io.open(ps, "w", encoding="utf-8", newline="\n").write(ps_exp)
print("experiment shaders written (originals in .keep)")
