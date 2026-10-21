// blur_down.hlsl —— 4-tap 角点下采样（输出尺寸 = 输入的一半）
Texture2D<float4> gSrc : register(t0);
SamplerState      gLin : register(s0);
cbuffer CB : register(b0) { float4 gTexel; };   // xy = 1/srcSize

float4 ps_main(float4 p : SV_POSITION) : SV_Target
{
    float2 uv = p.xy * gTexel.xy / max(gTexel.zw, float2(1e-6, 1e-6)); // dst pixel -> source UV
    float2 o  = gTexel.xy * 0.5;
    return (gSrc.SampleLevel(gLin, uv + float2(-o.x, -o.y), 0)
          + gSrc.SampleLevel(gLin, uv + float2( o.x, -o.y), 0)
          + gSrc.SampleLevel(gLin, uv + float2(-o.x,  o.y), 0)
          + gSrc.SampleLevel(gLin, uv + float2( o.x,  o.y), 0)) * 0.25;
}
