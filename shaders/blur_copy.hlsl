Texture2D<float4> gSrc : register(t0);
SamplerState gLin : register(s0);
cbuffer CB : register(b0) { float4 gTexel; float gWeightCenter; };

float4 ps_main(float4 p : SV_POSITION) : SV_Target
{
    return gSrc.SampleLevel(gLin, p.xy * gTexel.xy, 0);
}
