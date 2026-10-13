// blur_vs.hlsl —— 全屏三角形顶点着色器

float4 vs_main(uint vid : SV_VertexID) : SV_Position
{
    static const float2 vertices[3] = {
        float2(-1.0, -1.0),
        float2(-1.0,  3.0),
        float2( 3.0, -1.0)
    };
    return float4(vertices[vid], 0.0, 1.0);
}
