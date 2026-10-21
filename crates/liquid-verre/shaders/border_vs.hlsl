// border_vs.hlsl —— 缩放反馈 border 的独立叠加绘制（需求 F2.1 / F2.2）
//
// 为什么是独立 pass 而不是塞进 glass_ps：
//   border 是「盖在玻璃上面的一个元素」。塞进液态着色器会迫使它去压制液态着色器
//   原有的 hover 高光 / 鼠标渐变（否则直边会残留白线），既复杂又破坏逐位一致性。
//   独立 pass 之后 glass_ps 的液态分支与重构前一字不差，border 的几何也完全自控。
//
// 每个被 hover 且命中把手的块画一次（通常 0~1 次 draw），成本可忽略。

cbuffer BorderCb : register(b0)
{
    float4 b0; // 绘制矩形：左上角(画布px) / 尺寸(画布px)（= 块矩形 ⊕ 阴影外扩）
    float4 b1; // 块半尺寸 hs.xy / 圆角半径 r / AA 宽度
    float4 b2; // 直边掩码 (L, R, T, B)，0/1
    float4 b3; // 对角掩码 (TL, TR, BL, BR)，0/1
    float4 b4; // border 宽度(px) / 不透明度 / - / -
    float4 b5; // border 颜色 rgb / -
    float4 b6; // 画布尺寸(px)
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
    float2 canvasPx = b0.xy + quad[vid] * b0.zw;
    float2 canvasSize = max(b6.xy, float2(1,1));
    float2 ndc;
    ndc.x = (canvasPx.x / canvasSize.x) * 2.0 - 1.0;
    ndc.y = 1.0 - (canvasPx.y / canvasSize.y) * 2.0;
    VSOut o;
    o.pos = float4(ndc, 0.0, 1.0);
    return o;
}
