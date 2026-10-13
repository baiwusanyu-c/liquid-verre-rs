// glass_ps.hlsl — liquid-glass-rs 视觉模型 + 折射渲染方案高性能架构
// Pass A 提供全屏共享模糊层；Pass B 每块只做解析 SDF、LUT 折射和合成。
#define SHADOW_SIGMA 9.0
static const float3 ABSORB = float3(0.02, 0.005, 0.015);

Texture2D<float4> gCapture : register(t0);
Texture2D<float4> gFrost   : register(t1);
Texture2D<float4> gSoft    : register(t2);
Texture2D<float4> gEnv16   : register(t3);
StructuredBuffer<float2> gLut : register(t4);
Texture2D<float4> gBlur0   : register(t5);
Texture2D<float4> gBlur1   : register(t6);
Texture2D<float4> gBlur2   : register(t7);
Texture2D<float4> gBlur3   : register(t8);
SamplerState gSamp : register(s0);

cbuffer GlassParams : register(b0)
{
    float4 c0; float4 c1; float4 c2; float4 c3; float4 c4; float4 c5;
    float4 c6; float4 c7; float4 c8; float4 c9; float4 c10; float4 c11;
    float4 c12; float4 c13; float4 c14; float4 c15; float4 c16; float4 c17;
};

float SdRoundBox(float2 p, float2 hs, float r)
{
    r = min(r, min(hs.x, hs.y));
    float2 q = abs(p) - (hs - r);
    float2 m = max(q, 0.0);
    return length(m) - r + min(max(q.x, q.y), 0.0);
}
float2 SdRoundBoxGrad(float2 p, float2 hs, float r)
{
    r = min(r, min(hs.x, hs.y));
    float2 q = abs(p) - (hs - r), m = max(q, 0.0);
    float l = length(m);
    if (l > 1e-5) return (m / l) * sign(p);
    return (q.x > q.y) ? float2(sign(p.x), 0) : float2(0, sign(p.y));
}
float SdSquircle(float2 p, float2 hs, float n)
{
    float2 d = abs(p) / max(hs, float2(1e-4, 1e-4));
    float k = pow(pow(max(d.x,1e-6),n)+pow(max(d.y,1e-6),n),1.0/n);
    return (k-1.0)*min(hs.x,hs.y)*c15.y;
}
float2 SdSquircleGrad(float2 p,float2 hs,float n)
{
    float2 d=abs(p)/max(hs,float2(1e-4,1e-4));
    float sum=pow(max(d.x,1e-6),n)+pow(max(d.y,1e-6),n);
    float sp=pow(max(sum,1e-6),1.0/n-1.0);
    return normalize(sign(p)*pow(max(d,1e-6),n-1.0)/hs*sp+1e-8);
}
float Shape(float2 p,float2 hs){return c15.x<.5?SdRoundBox(p,hs,c3.y):SdSquircle(p,hs,c3.x);}
float2 ShapeGrad(float2 p,float2 hs){return c15.x<.5?SdRoundBoxGrad(p,hs,c3.y):SdSquircleGrad(p,hs,c3.x);}

float2 LutSample(float u)
{
    float f=saturate(u)*255.0; int i=(int)f; float t=f-i;
    return lerp(gLut[min(i,255)],gLut[min(i+1,255)],t);
}
float3 SamplePyramidLevel(int level,float2 uv)
{
    uv=saturate(uv);
    if(level<=0)return gBlur0.SampleLevel(gSamp,uv,0).rgb;
    if(level==1)return gBlur1.SampleLevel(gSamp,uv,0).rgb;
    if(level==2)return gBlur2.SampleLevel(gSamp,uv,0).rgb;
    return gBlur3.SampleLevel(gSamp,uv,0).rgb;
}
float3 SamplePyramid(float2 uv,float radiusPx)
{
    float lod=clamp(log2(max(radiusPx,3.0)/3.0),0.0,3.0);
    int i0=(int)floor(lod); float f=lod-i0;
    return lerp(SamplePyramidLevel(i0,uv),SamplePyramidLevel(min(i0+1,3),uv),f);
}
float3 SampleFrost(float2 uv,float radiusPx)
{
    uv=saturate(uv);
    if(radiusPx<=5.0)return gSoft.SampleLevel(gSamp,uv,0).rgb;
    if(radiusPx>=18.0)return gFrost.SampleLevel(gSamp,uv,0).rgb;
    float t=smoothstep(5.0,18.0,radiusPx);
    return lerp(gSoft.SampleLevel(gSamp,uv,0).rgb,gFrost.SampleLevel(gSamp,uv,0).rgb,t);
}
float FresnelSchlick(float c,float f0){float x=saturate(1-c),x2=x*x;return f0+(1-f0)*x2*x2*x;}
float GaussianTail(float distancePx,float sigma)
{
    float x=max(distancePx,0)/(max(sigma,.001)*1.41421356),t=1/(1+.3275911*x);
    float erfValue=1-(((((1.061405429*t-1.453152027)*t+1.421413741)*t-.284496736)*t+.254829592)*t)*exp(-x*x);
    return .5*(1-erfValue);
}
float GradientAlpha(float position,float stopA,float stopB,float alphaA,float alphaB)
{
    if(position<=stopA)return lerp(0,alphaA,saturate(position/max(stopA,.0001)));
    if(position<=stopB)return lerp(alphaA,alphaB,saturate((position-stopA)/max(stopB-stopA,.0001)));
    return lerp(alphaB,0,saturate((position-stopB)/max(1-stopB,.0001)));
}
float3 OverlayWhite(float3 b){return float3(b.r<=.5?b.r*2:1,b.g<=.5?b.g*2:1,b.b<=.5?b.b*2:1);}
float3 CssSaturate(float3 v,float a)
{
    return float3(dot(v,float3(.213+.787*a,.715-.715*a,.072-.072*a)),
                  dot(v,float3(.213-.213*a,.715+.285*a,.072-.072*a)),
                  dot(v,float3(.213-.213*a,.715-.715*a,.072+.928*a)));
}
float2 ReferenceDisplacement(float2 local,float2 lensSize,float scalePx)
{
    float2 uv=local/max(lensSize,float2(1,1))+.5,center=uv-.5;
    float mapDistance=SdRoundBox(center,float2(.3,.2),.6);
    float displacement=smoothstep(.8,0,mapDistance-.15),scaled=smoothstep(0,1,displacement);
    float2 rawOffset=center*(scaled-1)*lensSize;
    return clamp(rawOffset/max(lensSize.x*.3246682,1),-.5,.5)*scalePx;
}

float4 ps_main(float4 vpos:SV_POSITION):SV_Target
{
    float2 canvasPx=vpos.xy;
    float2 local=canvasPx-c16.xy-c0.zw*.5;
    float margin=c2.w;
    float2 hs=max(c0.zw*.5-margin,float2(1,1)),lensSize=hs*2;
    float sd=Shape(local,hs); float2 gr=ShapeGrad(local,hs);
    float aaHalf=max(c2.z*.5,.5);
    float glassCoverage=smoothstep(aaHalf,-aaHalf,sd);
    float shadow=GaussianTail(sd,SHADOW_SIGMA)*c6.w*lerp(1,.45,c5.x);
    if(glassCoverage<=0&&shadow<=.004)return float4(0,0,0,0);

    float insideDistance=max(-sd,0);
    float bevelWidth=lerp(max(c3.w,1),max(min(hs.x,hs.y),1),saturate(c3.z));
    float u=saturate(insideDistance/max(bevelWidth,1e-3));
    float2 lut=LutSample(u); float slope=min(lut.x,c15.z)*(1-c16.w),tan2=lut.y*(1-c16.w);
    float3 N=normalize(float3(gr*slope,1)),V=float3(0,0,1);

    float2 screenPx=canvasPx+c17.xy;
    float2 sampleScreen=screenPx+c13.xy*c13.z;
    float2 uv0=(sampleScreen-c1.xy)/c1.zw,texel=1/max(c1.zw,float2(1,1));

    float2 physicalOffset=-gr*(tan2*c4.y*c4.z);
    float2 referenceOffset=ReferenceDisplacement(local,lensSize,c4.y*c4.z*3.85);
    float2 offsetPx=lerp(physicalOffset,referenceOffset,.72);
    float edgeWeight=1-smoothstep(0,bevelWidth,insideDistance); offsetPx*=edgeWeight;
    float dk=c4.w*.08*(1-u);
    float2 uvR=saturate(uv0+offsetPx*(1-dk)*texel),uvG=saturate(uv0+offsetPx*texel),uvB=saturate(uv0+offsetPx*(1+dk)*texel);
    float3 sharp=float3(gCapture.SampleLevel(gSamp,uvR,0).r,gCapture.SampleLevel(gSamp,uvG,0).g,gCapture.SampleLevel(gSamp,uvB,0).b);

    float prog=lerp(1,pow(u,max(c5.w,.01)),step(.001,c5.w));
    float blurPx=lerp(c5.z,c5.y,prog)*(1-c5.x);
    float3 blurred=float3(SampleFrost(uvR,blurPx).r,SampleFrost(uvG,blurPx).g,SampleFrost(uvB,blurPx).b);
    // Reference keeps a clean center only for content; the backdrop itself remains
    // frosted. Keep edge refraction clear, but do not let edgeAlwaysSharp turn the
    // whole lens back into an unblurred desktop copy.
    float edgeSharp=pow(1-u,2)*edgeWeight;
    float sharpMix=saturate(c5.x+c10.y*(1-c5.x)*edgeSharp*.35);
    float3 col=lerp(blurred,sharp,sharpMix);
    col=saturate(CssSaturate(col,1.40));

    float pathLen=(1-(1-u)*.5)*c4.y*.02; col*=exp(-ABSORB*pathLen*c10.w);
    float3 env=gEnv16.SampleLevel(gSamp,saturate(uv0),0).rgb;
    float envL=dot(env,float3(.2126,.7152,.0722));
    float3 adapt=lerp(float3(1,1,1),1-env,c6.y*.35),tint=lerp(c7.rgb,c7.rgb*.6+env*.4,c14.w);
    col=lerp(col,tint*adapt,c6.x);

    float fres=FresnelSchlick(saturate(dot(N,V)),c9.z);
    float3 Lk=normalize(c8.xyz),Hk=normalize(Lk+V);
    float shininess=max(lerp(c9.y,c9.y*.12,c10.x),1);
    float specK=pow(saturate(dot(N,Hk)),shininess)*c8.w;
    float e=.01,converge=abs(LutSample(u+e).y-LutSample(u-e).y)/(2*e),caustic=saturate(converge*c10.z)*edgeWeight;
    float2 mouseVector=c11.xy-screenPx; float md2=dot(mouseVector,mouseVector),invR2=1/max(2*c11.z*c11.z,1e-3),fall=exp(-md2*invR2);
    float3 Lm=normalize(float3(mouseVector/max(c12.x,1),1)),Hm=normalize(Lm+V);
    float glint=pow(saturate(dot(N,Hm)),shininess)*fall*c11.w*edgeWeight;
    // Reference hover is the moving border gradient, not a rectangular radial
    // wash over the whole card. Keep only the physical edge glint here.
    float hoverGain=saturate(c12.y*.72+fall*.28);
    float specBoost=lerp(1,1.9,c6.y*saturate(1-envL*1.6));
    col+=c14.xyz*((specK*fres*c9.x+caustic*fres*.35)*specBoost+glint*hoverGain*(1+.35*c12.z));

    float2 blockCenterScreen=c16.xy+c0.zw*.5+c17.xy;
    float2 mouseOffset=c11.xy-blockCenterScreen;
    // liquid-glass-rs smooths this vector on CPU (0.14 EMA). Use the existing
    // drag-velocity/lag channel as a prediction term so the gradient follows the
    // pointer continuously without introducing a per-block CPU allocation.
    float2 lightOffset=mouseOffset-c13.xy*c13.z;
    float2 normalizedMouse=float2(lightOffset.x/max(lensSize.x,1),lightOffset.y/max(lensSize.y,1))*100;
    float angle=radians(135+normalizedMouse.x*1.2); float2 gradientDirection=float2(sin(angle),-cos(angle));
    float gradientExtent=max(dot(abs(gradientDirection),hs),.0001);
    float2 localTopLeft=local+hs;
    float gradientPosition=dot(localTopLeft-lensSize*.5,gradientDirection)/(gradientExtent*2)+.5;
    float stopA=clamp((33+normalizedMouse.y*.3)/100,.1,.9),stopB=clamp((66+normalizedMouse.y*.4)/100,.1,.9);
    float borderWidth=max(c9.w,1),borderMask=saturate(1-smoothstep(0,borderWidth,insideDistance))*glassCoverage;
    float insetDirection=saturate(.5-gr.y*.5),insetGlow=exp(-.5*pow(insideDistance/max(borderWidth,.001),2))*insetDirection*.25;
    float insetAlpha=saturate((1-smoothstep(.25,.75,insideDistance))*.5+insetGlow)*borderMask;
    float screenAlpha=saturate(GradientAlpha(gradientPosition,stopA,stopB,.12+abs(normalizedMouse.x)*.008,.4+abs(normalizedMouse.x)*.012))*borderMask*.2;
    screenAlpha=1-(1-screenAlpha)*(1-insetAlpha*.2); col=1-(1-col)*(1-screenAlpha);
    float overlayAlpha=saturate(GradientAlpha(gradientPosition,stopA,stopB,.32+abs(normalizedMouse.x)*.008,.6+abs(normalizedMouse.x)*.012))*borderMask;
    overlayAlpha=1-(1-overlayAlpha)*(1-insetAlpha); col=lerp(col,OverlayWhite(col),overlayAlpha);

    // Captured desktop is already inside col: replace the underlying desktop within
    // the glass shape. Partial alpha would double-composite sharp and blurred images.
    float glassAlpha=glassCoverage*c6.z;
    // Outside shadow is subtle premultiplied black. Inside the glass no shadow
    // contribution is allowed, preventing a gray/black rectangle around the lens.
    float shadowAlpha=shadow*(1-glassCoverage)*.45;
    float alpha=saturate(glassAlpha+shadowAlpha*(1-glassAlpha));
    return float4(col*glassAlpha,alpha);
}
