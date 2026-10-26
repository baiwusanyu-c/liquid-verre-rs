//! HLSL 编译与状态对象缓存（build.rs 已声明 shaders/ 依赖）。

use std::ffi::c_void;

use windows::core::{Interface, PCSTR};
use windows::Win32::Foundation::BOOL;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::ID3DBlob;
use windows::Win32::Graphics::Direct3D11::*;

pub const GLASS_VS: &str = include_str!("../../shaders/glass_vs.hlsl");
pub const BLUR_VS: &str = include_str!("../../shaders/blur_vs.hlsl");
pub const GLASS_PS: &str = include_str!("../../shaders/glass_ps.hlsl");
pub const BLUR_DOWN: &str = include_str!("../../shaders/blur_down.hlsl");
pub const BLUR_UP: &str = include_str!("../../shaders/blur_up.hlsl");
pub const ENV_EMA: &str = include_str!("../../shaders/env_ema.hlsl");
pub const BORDER_VS: &str = include_str!("../../shaders/border_vs.hlsl");
pub const BORDER_PS: &str = include_str!("../../shaders/border_ps.hlsl");
pub const CONTENT_VS: &str = include_str!("../../shaders/content_vs.hlsl");
pub const CONTENT_PS: &str = include_str!("../../shaders/content_ps.hlsl");

pub fn compile(src: &str, entry: &str, target: &str) -> crate::error::Result<ID3DBlob> {
    let entry_c = std::ffi::CString::new(entry).unwrap();
    let target_c = std::ffi::CString::new(target).unwrap();
    let mut code: Option<ID3DBlob> = None;
    let mut err: Option<ID3DBlob> = None;
    let hr = unsafe {
        D3DCompile(
            src.as_ptr() as *const c_void,
            src.len(),
            PCSTR::null(),
            None,
            None,
            PCSTR::from_raw(entry_c.as_ptr() as *const u8),
            PCSTR::from_raw(target_c.as_ptr() as *const u8),
            0,
            0,
            &mut code,
            Some(&mut err),
        )
    };
    if let Err(e) = hr {
        let msg = err
            .map(|b| unsafe {
                String::from_utf8_lossy(std::slice::from_raw_parts(
                    b.GetBufferPointer() as *const u8,
                    b.GetBufferSize(),
                ))
                .into_owned()
            })
            .unwrap_or_else(|| e.to_string());
        return Err(format!("[{entry}] {msg}").into());
    }
    code.ok_or_else(|| crate::error::Error::from(format!("[{entry}] 编译未返回字节码")))
}

pub fn bytes(b: &ID3DBlob) -> &[u8] {
    unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) }
}

/// 全屏三角形顶点着色器 / 像素着色器对
pub struct SimpleShader {
    pub vs: ID3D11VertexShader,
    pub ps: ID3D11PixelShader,
}

/// VS 与 PS 来自不同文件时使用
pub fn make_shader_pair(
    device: &ID3D11Device,
    vs_src: &str,
    vs_entry: &str,
    ps_src: &str,
    ps_entry: &str,
) -> crate::error::Result<SimpleShader> {
    let vb = compile(vs_src, vs_entry, "vs_5_0")?;
    let pb = compile(ps_src, ps_entry, "ps_5_0")?;
    let vs = unsafe {
        let mut o = None;
        device
            .CreateVertexShader(bytes(&vb), None, Some(&mut o))
            .map_err(|e| format!("CreateVertexShader: {e}"))?;
        o.ok_or("顶点着色器为空")?
    };
    let ps = unsafe {
        let mut o = None;
        device
            .CreatePixelShader(bytes(&pb), None, Some(&mut o))
            .map_err(|e| format!("CreatePixelShader: {e}"))?;
        o.ok_or("像素着色器为空")?
    };
    Ok(SimpleShader { vs, ps })
}

/// 无混合 + 关闭深度/剔除的公共状态（Pass A 用）
pub fn make_opaque_states(device: &ID3D11Device) -> crate::error::Result<OpaqueStates> {
    let mut bl = D3D11_BLEND_DESC::default();
    bl.RenderTarget[0].BlendEnable = BOOL(0);
    bl.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8;
    let blend = unsafe {
        let mut o = None;
        device
            .CreateBlendState(&bl, Some(&mut o))
            .map_err(|e| e.to_string())?;
        o.ok_or("CreateBlendState")?
    };
    let rd = D3D11_RASTERIZER_DESC {
        FillMode: D3D11_FILL_SOLID,
        CullMode: D3D11_CULL_NONE,
        FrontCounterClockwise: BOOL(0),
        DepthBias: 0,
        DepthBiasClamp: 0.0,
        SlopeScaledDepthBias: 0.0,
        DepthClipEnable: BOOL(1),
        ScissorEnable: BOOL(1),
        MultisampleEnable: BOOL(0),
        AntialiasedLineEnable: BOOL(0),
    };
    let raster = unsafe {
        let mut o = None;
        device
            .CreateRasterizerState(&rd, Some(&mut o))
            .map_err(|e| e.to_string())?;
        o.ok_or("CreateRasterizerState")?
    };
    let dd = D3D11_DEPTH_STENCIL_DESC {
        DepthEnable: BOOL(0),
        DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ALL,
        DepthFunc: D3D11_COMPARISON_NEVER,
        StencilEnable: BOOL(0),
        StencilReadMask: 0,
        StencilWriteMask: 0,
        FrontFace: D3D11_DEPTH_STENCILOP_DESC::default(),
        BackFace: D3D11_DEPTH_STENCILOP_DESC::default(),
    };
    let depth = unsafe {
        let mut o = None;
        device
            .CreateDepthStencilState(&dd, Some(&mut o))
            .map_err(|e| e.to_string())?;
        o.ok_or("CreateDepthStencilState")?
    };
    Ok(OpaqueStates {
        blend,
        raster,
        depth,
    })
}

pub struct OpaqueStates {
    pub blend: ID3D11BlendState,
    pub raster: ID3D11RasterizerState,
    pub depth: ID3D11DepthStencilState,
}

/// 预乘 alpha 混合状态（Pass B 用）
pub fn make_premultiplied_blend(device: &ID3D11Device) -> crate::error::Result<ID3D11BlendState> {
    let mut bl = D3D11_BLEND_DESC::default();
    let rt = &mut bl.RenderTarget[0];
    rt.BlendEnable = BOOL(1);
    rt.SrcBlend = D3D11_BLEND_ONE;
    rt.DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
    rt.BlendOp = D3D11_BLEND_OP_ADD;
    rt.SrcBlendAlpha = D3D11_BLEND_ONE;
    rt.DestBlendAlpha = D3D11_BLEND_INV_SRC_ALPHA;
    rt.BlendOpAlpha = D3D11_BLEND_OP_ADD;
    rt.RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8;
    let mut o = None;
    unsafe {
        device
            .CreateBlendState(&bl, Some(&mut o))
            .map_err(|e| e.to_string())?
    };
    o.ok_or_else(|| crate::error::Error::from("CreateBlendState(premultiplied)"))
}

/// 线性 + CLAMP 采样器
pub fn make_linear_sampler(device: &ID3D11Device) -> crate::error::Result<ID3D11SamplerState> {
    let sd = D3D11_SAMPLER_DESC {
        Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
        MipLODBias: 0.0,
        MaxAnisotropy: 1,
        ComparisonFunc: D3D11_COMPARISON_NEVER,
        BorderColor: [0.0; 4],
        MinLOD: 0.0,
        MaxLOD: f32::MAX,
    };
    let mut o = None;
    unsafe {
        device
            .CreateSamplerState(&sd, Some(&mut o))
            .map_err(|e| e.to_string())?
    };
    o.ok_or_else(|| crate::error::Error::from("CreateSamplerState"))
}

/// 简单动态常量缓冲区
pub fn make_dynamic_cbuffer(
    device: &ID3D11Device,
    byte_width: u32,
) -> crate::error::Result<ID3D11Buffer> {
    let bd = D3D11_BUFFER_DESC {
        ByteWidth: byte_width,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let mut o = None;
    unsafe {
        device
            .CreateBuffer(&bd, None, Some(&mut o))
            .map_err(|e| e.to_string())?
    };
    o.ok_or_else(|| crate::error::Error::from("CreateBuffer(cbuffer)"))
}

/// 更新动态常量缓冲区
pub fn update_cbuffer<T: Copy>(
    ctx: &ID3D11DeviceContext,
    buf: &ID3D11Buffer,
    value: &T,
) -> crate::error::Result<()> {
    let mut m = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe {
        ctx.Map(buf, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut m))
            .map_err(|e| format!("Map: {e}"))?;
        std::ptr::copy_nonoverlapping(
            value as *const T as *const u8,
            m.pData as *mut u8,
            std::mem::size_of::<T>(),
        );
        ctx.Unmap(buf, 0);
    }
    Ok(())
}

/// RT 描述（Pass A 复用）
#[derive(Clone)]
pub struct RenderTarget {
    pub tex: ID3D11Texture2D,
    pub rtv: ID3D11RenderTargetView,
    pub srv: ID3D11ShaderResourceView,
    pub w: i32,
    pub h: i32,
}

pub fn make_rt(
    device: &ID3D11Device,
    w: i32,
    h: i32,
    format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT,
) -> crate::error::Result<RenderTarget> {
    use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
    let td = D3D11_TEXTURE2D_DESC {
        Width: w.max(1) as u32,
        Height: h.max(1) as u32,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut tex = None;
    unsafe {
        device
            .CreateTexture2D(&td, None, Some(&mut tex))
            .map_err(|e| e.to_string())?
    };
    let tex = tex.ok_or("CreateTexture2D")?;
    let mut rtv = None;
    unsafe {
        device
            .CreateRenderTargetView(&tex, None, Some(&mut rtv))
            .map_err(|e| e.to_string())?
    };
    let mut srv = None;
    unsafe {
        device
            .CreateShaderResourceView(&tex, None, Some(&mut srv))
            .map_err(|e| e.to_string())?
    };
    Ok(RenderTarget {
        tex,
        rtv: rtv.ok_or("CreateRenderTargetView")?,
        srv: srv.ok_or("CreateShaderResourceView")?,
        w: w.max(1),
        h: h.max(1),
    })
}

#[allow(dead_code)]
fn _iface_guard() {
    let _ = <ID3D11Device as Interface>::IID;
    let _ = BOOL(0);
}
