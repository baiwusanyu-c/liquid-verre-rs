// border_ps.hlsl —— 加粗白色 border 的形状定义（需求 F2.1 / F2.2）
//
//   直边（F2.1）：band × 「只在圆角切点之间」的窗 —— border 画到圆角为止，**不拐弯**。
//   对角（F2.2）：band × 「只在圆角象限」的窗 —— 用 SDF 内侧距离做 band，
//                 所以在圆角处**自然沿弧**走（参考图红框里的形状）。
//
// 两种掩码互斥由 Rust 侧保证（`ResizeEdges::to_shader_edges` 在对角时全 0、
// `to_shader_corners` 在单边时全 0，core/handle.rs 有单测钉住）。
//
// 输出是 premultiplied alpha，用与玻璃相同的 blend state 叠在玻璃之上。

cbuffer BorderCb : register(b0)
{
    float4 b0;
    float4 b1;
    float4 b2;
    float4 b3;
    float4 b4;
    float4 b5;
    float4 b6;
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
    float2 hs = b1.xy;
    float r = min(b1.z, min(hs.x, hs.y));
    float aa = max(b1.w, 0.5);
    float bw = max(b4.x, 1e-3);

    // 绘制矩形是块矩形 ⊕ 对称外扩，因此它的中心 == 块中心。
    float2 local = vpos.xy - b0.xy - b0.zw * 0.5;
    float sd = SdRoundBox(local, hs, r);
    // 只在玻璃形状内（含 AA 边）：否则 border 会漏到形状外变成白色实心矩形。
    float cov = smoothstep(aa, -aa, sd);
    if (cov <= 0.0) return float4(0, 0, 0, 0);
    // 实心芯 + 外沿 1px AA：历史写法 1-smoothstep(0,bw,t) 从第 1 像素就开始渐隐，
    // 实测 bw=5 时人眼只看到约 2px 的白线（用户反馈「不明显」的根因）。
    // border 应当读作一条实线：t < bw-aaBand 全不透明，最后 aaBand 像素做 AA。
    float aaBand = min(aa, bw * 0.5);
    float inside = max(-sd, 0.0);
    float band = 1.0 - smoothstep(bw - aaBand, bw, inside);

    float selL = step(local.x, 0.0), selR = step(0.0, local.x);
    float selT = step(local.y, 0.0), selB = step(0.0, local.y);

    // --- 直边：靠近某条边 bw 像素内，且没越过圆角切点 ---
    float bandX = 1.0 - smoothstep(bw - aaBand, bw, hs.x - abs(local.x));
    float bandY = 1.0 - smoothstep(bw - aaBand, bw, hs.y - abs(local.y));
    float winY = 1.0 - smoothstep(hs.y - r - aa, hs.y - r + aa, abs(local.y));
    float winX = 1.0 - smoothstep(hs.x - r - aa, hs.x - r + aa, abs(local.x));
    float edge = bandX * winY * (b2.x * selL + b2.y * selR)
               + bandY * winX * (b2.z * selT + b2.w * selB);

    // --- 对角：q>0 = 落在圆角象限；band 用 SDF 内侧距离，因此沿弧 ---
    float2 q = abs(local) - (hs - r);
    float cornerWin = smoothstep(-aa, aa, q.x) * smoothstep(-aa, aa, q.y);
    float corner = band * cornerWin *
        (b3.x * selL * selT + b3.y * selR * selT + b3.z * selL * selB + b3.w * selR * selB);

    float a = saturate(max(edge, corner)) * b4.y * cov;
    if (a <= 0.0) return float4(0, 0, 0, 0);
    return float4(b5.rgb * a, a);
}
