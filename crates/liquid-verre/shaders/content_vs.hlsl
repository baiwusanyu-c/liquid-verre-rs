// content_vs.hlsl -- content layer overlay vertex shader (P0-1, FR3/FR4).
//
// Same fullscreen-triangle pattern as border_vs.hlsl: the quad is the block
// rect in canvas pixels, mapped to NDC via the canvas size. Runs interleaved
// right after each block's glass draw inside draw_blocks (see compositor /
// glass_renderer for why per-block interleaving is mandatory).
//
// No TEXCOORD output: the PS derives texel indices from SV_Position directly
// (1:1 texture mapping, no filtering), so no sampler is needed at all.

cbuffer ContentCb : register(b1)
{
    float4 c0; // block rect in canvas px: left/top (rounded) / width / height
    float4 c1; // texture w,h / effective radius (px) / AA width
    float4 c2; // canvas size (px) / - / -
};

struct VSOut
{
    float4 pos : SV_Position;
};

VSOut vs_main(uint vid : SV_VertexID)
{
    static const float2 quad[6] = {
        float2(0,0), float2(1,0), float2(0,1),
        float2(0,1), float2(1,0), float2(1,1)
    };
    float2 canvasPx = c0.xy + quad[vid] * c0.zw;
    float2 canvasSize = max(c2.xy, float2(1, 1));
    float2 ndc;
    ndc.x = (canvasPx.x / canvasSize.x) * 2.0 - 1.0;
    ndc.y = 1.0 - (canvasPx.y / canvasSize.y) * 2.0;
    VSOut o;
    o.pos = float4(ndc, 0.0, 1.0);
    return o;
}
