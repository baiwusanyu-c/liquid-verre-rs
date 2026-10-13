// 备用：独立 pass 版环境色探针（当前 shader 内联 EMA，未接入管线）。
// reduce_env.hlsl —— 1x1 环境色探针（GPU 侧，不回读）
// 对 gBlur3（1/16 层）做一次全局均值，输出 1x1，供下一帧 EMA 使用。
Texture2D<float4> gSrc : register(t0);
SamplerState      gLin : register(s0);
cbuffer CB : register(b0) { float4 gSrcSize; float4 gPrev; };

float4 ps_main(float4 p : SV_POSITION) : SV_Target
{
    // 固定 16x16 网格采样，足够稳定且极廉价
    float3 sum = 0;
    [unroll]
    for (int y = 0; y < 16; ++y)
    {
        [unroll]
        for (int x = 0; x < 16; ++x)
        {
            float2 uv = (float2(x, y) + 0.5) / 16.0;
            sum += gSrc.SampleLevel(gLin, uv, 0).rgb;
        }
    }
    float3 now = sum / 256.0;
    float3 ema = lerp(gPrev.rgb, now, 0.12);   // τ ≈ 150ms @60fps
    return float4(ema, 1.0);
}
