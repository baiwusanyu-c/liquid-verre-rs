// glass_vs.hlsl —— 单画布模式：为每个块绘制一个屏幕对齐的矩形（像素坐标 -> 裁剪空间）
// c16.xy = 块窗口左上角（画布像素坐标）  c0.zw = 块窗口尺寸（块 + 2*shadowMargin）
// c17.zw = 画布尺寸（像素）

cbuffer GlassParams : register(b0)
{
    float4 c0;
    float4 c1;
    float4 c2;
    float4 c3;
    float4 c4;
    float4 c5;
    float4 c6;
    float4 c7;
    float4 c8;
    float4 c9;
    float4 c10;
    float4 c11;
    float4 c12;
    float4 c13;
    float4 c14;
    float4 c15;
    float4 c16;
    float4 c17;
};

struct VSOut
{
    float4 pos   : SV_Position;
    float2 local : TEXCOORD0;
};

VSOut vs_main(uint vid : SV_VertexID)
{
    static const float2 quad[6] = {
        float2(0,0), float2(1,0), float2(0,1),
        float2(0,1), float2(1,0), float2(1,1)
    };
    float2 t = quad[vid];
    float2 canvasPx = c16.xy + t * c0.zw;

    float2 canvasSize = max(c17.zw, float2(1,1));
    float2 ndc;
    ndc.x = (canvasPx.x / canvasSize.x) * 2.0 - 1.0;
    ndc.y = 1.0 - (canvasPx.y / canvasSize.y) * 2.0;

    VSOut o;
    o.pos = float4(ndc, 0.0, 1.0);
    o.local = t * c0.zw;
    return o;
}
