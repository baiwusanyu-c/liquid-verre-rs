import io, os
R = r"D:\github-project\liquid-verre-rs"
ps_path = os.path.join(R, r"crates\liquid-verre\shaders\content_ps.hlsl")
src = """// content_ps.hlsl -- content layer pixel shader (P0-1, FR4).
//
// Texture t9 is the block's premultiplied BGRA8 content, anchored 1:1 at the
// block rect's top-left. Texels are fetched by integer Load (no sampler, no
// filtering -- FR1's 1:1 promise and the pixel-exact forensics both require
// nearest-neighbour on exact pixel centers).
//
// The round-corner mask uses SdRoundBox with the radius passed in via c1.z.
// That radius has ONE source: core::block::Block::effective_radius, clamped
// the same way as the glass pass computes p.corner_radius -- this shader does
// NOT carry a second radius formula (C5).
//
// Out-of-texture pixels (c1.x/y smaller than the block rect) are transparent:
// "missing content stays transparent", never edge-clamped garbage.

Texture2D texContent : register(t9);

cbuffer ContentCb : register(b1)
{
    float4 c0; // block rect in canvas px: left/top (rounded) / width / height
    float4 c1; // texture w,h / effective radius (px) / AA width
    float4 c2; // canvas size (px) / - / -
};

float SdRoundBox(float2 p, float2 hs, float r)
{
    r = min(r, min(hs.x, hs.y));
    float2 q = abs(p) - (hs - r);
    float2 m = max(q, 0.0);
    return length(m) - r + min(max(q.x, q.y), 0.0);
}

float4 ps_main(float4 vpos : SV_Position) : SV_Target
{
    // Texel index == integer pixel offset from the block rect origin.
    // vpos.xy is the pixel center (X+0.5, Y+0.5); c0.xy is the rounded rect
    // origin, so int(vpos.xy - c0.xy) == the canvas pixel offset == texel idx.
    int2 t = int2(vpos.xy - c0.xy);
    if (t.x < 0 || t.y < 0 || t.x >= (int)c1.x || t.y >= (int)c1.y)
    {
        return float4(0, 0, 0, 0);
    }
    float4 tex = texContent.Load(int3(t, 0));

    // Round-corner mask, identical formula to border_ps / glass shape.
    float2 hs = c0.zw * 0.5;
    float r = min(c1.z, min(hs.x, hs.y));
    float aa = max(c1.w, 0.5);
    float2 local = vpos.xy - c0.xy - hs;
    float sd = SdRoundBox(local, hs, r);
    float cov = smoothstep(aa, -aa, sd);
    if (cov <= 0.0)
        return float4(0, 0, 0, 0);

    // Premultiplied alpha output; the glass pass blend state
    // (ONE / INV_SRC_ALPHA) composites it over this block's glass.
    return float4(tex.rgb * cov, tex.a * cov);
}
"""
with io.open(ps_path, "w", encoding="utf-8", newline="\n") as f:
    f.write(src)
# clean leftover experiment artifacts
for junk in (ps_path + ".orig", ps_path + ".keep",
             os.path.join(R, r"crates\liquid-verre\shaders\content_vs.hlsl.keep")):
    if os.path.exists(junk):
        os.unlink(junk)
os.utime(os.path.join(R, r"crates\liquid-verre\shaders\content_vs.hlsl"), None)
os.utime(ps_path, None)
print("real content_ps.hlsl restored; shaders touched")
