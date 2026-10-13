// blur_up.hlsl —— 8-tap tent 上采样（输出尺寸 = 输入的 2 倍）
Texture2D<float4> gSrc : register(t0);
SamplerState      gLin : register(s0);
cbuffer CB : register(b0) { float4 gTexel; float gWeightCenter; };
// gTexel.xy = 1/srcW, 1/srcH（源在块像素空间）

float4 ps_main(float4 p : SV_POSITION) : SV_Target
{
    float2 uv = p.xy * gTexel.xy / max(gTexel.zw, float2(1e-6, 1e-6));
    float2 o  = gTexel.xy;
    float4 s  = gSrc.SampleLevel(gLin, uv + float2(-o.x,  0   ), 0) * 2.0
              + gSrc.SampleLevel(gLin, uv + float2( o.x,  0   ), 0) * 2.0
              + gSrc.SampleLevel(gLin, uv + float2( 0,   -o.y ), 0) * 2.0
              + gSrc.SampleLevel(gLin, uv + float2( 0,    o.y ), 0) * 2.0
              + gSrc.SampleLevel(gLin, uv + float2(-o.x, -o.y ), 0)
              + gSrc.SampleLevel(gLin, uv + float2( o.x, -o.y ), 0)
              + gSrc.SampleLevel(gLin, uv + float2(-o.x,  o.y ), 0)
              + gSrc.SampleLevel(gLin, uv + float2( o.x,  o.y ), 0);
    return s / 12.0;
}
