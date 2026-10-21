// env_ema.hlsl —— 1/16 环境色探针的时域平滑（方案 12.6 / 风险条目 R-20 必修项）
//
// 自适应染色若直接用当前帧的均值，在动态背景（视频、动画壁纸、闪烁光标）上会
// 逐帧抖动 -> 玻璃染色高频闪烁。这里对 1/16 层做 GPU 侧 ping-pong EMA：
//   out = lerp(prev, cur, alpha),  alpha = 1 - exp(-dt / TAU),  TAU ≈ 0.15 s
// 零回读、零 CPU、无额外延迟。
//
// 静态桌面下 cur 恒定，EMA 一步收敛到 cur（lerp(x,x,a) == x 精确成立），
// 因此输出与"不做 EMA"逐位一致 —— 这也是它能通过逐像素自证的原因。
Texture2D<float4> gCur  : register(t0);   // 本帧 1/16（down[3]）
Texture2D<float4> gPrev : register(t1);   // 上一帧的 EMA 结果
SamplerState      gLin  : register(s0);
cbuffer CB : register(b0)
{
    float4 gTexel;      // xy = 1/srcSize, zw = dst/src 比例
    float  gAlpha;      // 1 - exp(-dt/TAU)
    float  gFirst;      // >0.5 表示没有可用历史，直接落当前值
};

float4 ps_main(float4 p : SV_POSITION) : SV_Target
{
    float2 uv = p.xy * gTexel.xy / max(gTexel.zw, float2(1e-6, 1e-6));
    float3 cur  = gCur.SampleLevel(gLin, uv, 0).rgb;
    float  a    = gFirst > 0.5 ? 1.0 : saturate(gAlpha);
    if (a >= 1.0)
    {
        return float4(cur, 1.0);
    }
    float3 prev = gPrev.SampleLevel(gLin, uv, 0).rgb;
    return float4(lerp(prev, cur, a), 1.0);
}
