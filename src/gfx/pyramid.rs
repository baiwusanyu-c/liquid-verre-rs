//! Pass A：全屏共享模糊金字塔（方案 5.2）。
//! 4 级 dual-filter 金字塔，一帧只算一次，与块数无关。
//!
//!   gBlur0 = 1/2   gBlur1 = 1/4   gBlur2 = 1/8   gBlur3 = 1/16
//!   下采样：capture → 1/2 → 1/4 → 1/8 → 1/16
//!   上采样：1/16 → 1/8 → 1/4 → 1/2（覆盖式 tent 重建，避免锯齿方块）

use windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;

use crate::gfx::shaders::RenderTarget;
use crate::gfx::shaders::{
    make_dynamic_cbuffer, make_linear_sampler, make_opaque_states, make_rt, make_shader_pair,
    update_cbuffer, OpaqueStates, SimpleShader,
};

const TOPO_TRIANGLE: D3D_PRIMITIVE_TOPOLOGY = D3D_PRIMITIVE_TOPOLOGY(4);

#[repr(C)]
#[derive(Clone, Copy)]
struct BlurCb {
    texel: [f32; 4],
    weight_center: f32,
    _pad: [f32; 3],
}

pub const LEVELS: usize = 4;

pub struct Pyramid {
    /// down[0]=1/2 ... down[3]=1/16
    down: Vec<RenderTarget>,
    /// up[0]=1/2 ... up[2]=1/8（上采样重建结果）
    up: Vec<RenderTarget>,
    /// 强模糊 ping-pong 链：两个 1/2 分辨率 RT，重复 tent pass 近似
    /// liquid-glass-rs 的 D2D GaussianBlur sigma≈20，同时仍为全屏共享资源。
    frost_half: Vec<RenderTarget>,
    /// 块分辨率的强模糊 / 轻模糊（整屏尺寸，Pass B 直接采样）
    pub frost: Option<RenderTarget>,
    pub soft: Option<RenderTarget>,
    /// 1/16 环境色探针（整屏缩小，用于自适应染色）
    pub env16: Option<RenderTarget>,
    down_shader: SimpleShader,
    up_shader: SimpleShader,
    copy_shader: SimpleShader,
    cb: ID3D11Buffer,
    sampler: ID3D11SamplerState,
    states: OpaqueStates,
    width: i32,
    height: i32,
}

impl Pyramid {
    pub fn new(device: &ID3D11Device) -> Result<Self, String> {
        let down_shader = make_shader_pair(
            device,
            crate::gfx::shaders::BLUR_VS,
            "vs_main",
            crate::gfx::shaders::BLUR_DOWN,
            "ps_main",
        )?;
        let up_shader = make_shader_pair(
            device,
            crate::gfx::shaders::BLUR_VS,
            "vs_main",
            crate::gfx::shaders::BLUR_UP,
            "ps_main",
        )?;
        let copy_shader = make_shader_pair(
            device,
            crate::gfx::shaders::BLUR_VS,
            "vs_main",
            crate::gfx::shaders::BLUR_COPY,
            "ps_main",
        )?;
        let cb = make_dynamic_cbuffer(device, std::mem::size_of::<BlurCb>() as u32)?;
        let sampler = make_linear_sampler(device)?;
        let states = make_opaque_states(device)?;
        Ok(Self {
            down: Vec::new(),
            up: Vec::new(),
            frost_half: Vec::new(),
            frost: None,
            soft: None,
            env16: None,
            down_shader,
            up_shader,
            copy_shader,
            cb,
            sampler,
            states,
            width: 0,
            height: 0,
        })
    }

    pub fn width(&self) -> i32 {
        self.width
    }
    pub fn height(&self) -> i32 {
        self.height
    }

    /// 第 i 级模糊层（0=1/2 ... 3=1/16）
    /// 使用 down 链（下采样结果）而不是 up 链，确保数据正确
    pub fn level(&self, i: usize) -> Option<&RenderTarget> {
        match i {
            0 => self.down.get(0),
            1 => self.down.get(1),
            2 => self.down.get(2),
            3 => self.down.get(3),
            _ => None,
        }
    }

    pub fn resize(&mut self, device: &ID3D11Device, w: i32, h: i32) -> Result<(), String> {
        let (w, h) = (w.max(4), h.max(4));
        if self.width == w && self.height == h && self.down.len() == LEVELS {
            return Ok(());
        }
        self.width = w;
        self.height = h;
        self.down.clear();
        self.up.clear();
        for k in 0..LEVELS {
            let (dw, dh) = ((w >> (k + 1)).max(1), (h >> (k + 1)).max(1));
            self.down
                .push(make_rt(device, dw, dh, DXGI_FORMAT_B8G8R8A8_UNORM)?);
        }
        for k in 0..(LEVELS - 1) {
            let (uw, uh) = ((w >> (k + 1)).max(1), (h >> (k + 1)).max(1));
            self.up
                .push(make_rt(device, uw, uh, DXGI_FORMAT_B8G8R8A8_UNORM)?);
        }
        // 块分辨率模糊链：整屏 → 1/2 → 1/2（两次，等效 σ≈20px）→ 上采样回整屏
        self.frost_half.clear();
        for _ in 0..2 {
            let (hw, hh) = ((w >> 1).max(1), (h >> 1).max(1));
            self.frost_half
                .push(make_rt(device, hw, hh, DXGI_FORMAT_B8G8R8A8_UNORM)?);
        }
        self.frost = Some(make_rt(device, w, h, DXGI_FORMAT_B8G8R8A8_UNORM)?);
        self.soft = Some(make_rt(device, w, h, DXGI_FORMAT_B8G8R8A8_UNORM)?);
        self.env16 = Some(make_rt(
            device,
            self.down[LEVELS - 1].w,
            self.down[LEVELS - 1].h,
            DXGI_FORMAT_B8G8R8A8_UNORM,
        )?);
        Ok(())
    }

    fn bind(&self, ctx: &ID3D11DeviceContext) {
        unsafe {
            ctx.IASetInputLayout(None);
            ctx.IASetPrimitiveTopology(TOPO_TRIANGLE);
            ctx.OMSetBlendState(&self.states.blend, Some(&[0.0; 4]), 0xffff_ffff);
            ctx.OMSetDepthStencilState(&self.states.depth, 0);
            ctx.RSSetState(&self.states.raster);
            ctx.VSSetShader(&self.down_shader.vs, None);
            ctx.PSSetShader(&self.down_shader.ps, None);
            ctx.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
        }
    }

    fn draw(
        &self,
        ctx: &ID3D11DeviceContext,
        src: &ID3D11ShaderResourceView,
        dst: &RenderTarget,
        src_w: i32,
        src_h: i32,
    ) {
        self.draw_with(ctx, &self.down_shader.ps, src, dst, src_w, src_h);
    }

    /// 使用专用像素着色器绘制一个金字塔步骤。
    fn draw_with(
        &self,
        ctx: &ID3D11DeviceContext,
        ps: &ID3D11PixelShader,
        src: &ID3D11ShaderResourceView,
        dst: &RenderTarget,
        src_w: i32,
        src_h: i32,
    ) {
        let cb_value = BlurCb {
            texel: [
                1.0 / src_w.max(1) as f32,
                1.0 / src_h.max(1) as f32,
                dst.w as f32 / src_w.max(1) as f32,
                dst.h as f32 / src_h.max(1) as f32,
            ],
            weight_center: 0.5,
            _pad: [0.0; 3],
        };
        if update_cbuffer(ctx, &self.cb, &cb_value).is_err() {
            return;
        }
        unsafe {
            // Pass B binds t0..t8. All nine slots must be cleared before any
            // pyramid texture is rebound as an RTV on the next frame.
            let null: [Option<ID3D11ShaderResourceView>; 9] =
                [None, None, None, None, None, None, None, None, None];
            ctx.PSSetShaderResources(0, Some(&null));
            ctx.OMSetRenderTargets(None, None);
            ctx.OMSetRenderTargets(Some(&[Some(dst.rtv.clone())]), None);
            ctx.ClearRenderTargetView(&dst.rtv, &[0.0, 0.0, 0.0, 1.0]);
            let vp = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: dst.w as f32,
                Height: dst.h as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            ctx.RSSetViewports(Some(&[vp]));
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
            ctx.PSSetShader(ps, None);
            ctx.PSSetShaderResources(0, Some(&[Some(src.clone())]));
            ctx.Draw(3, 0);
        }
    }

    /// 用专用 tent 上采样着色器重建到目标尺寸。
    fn draw_up(
        &self,
        ctx: &ID3D11DeviceContext,
        src: &ID3D11ShaderResourceView,
        dst: &RenderTarget,
        src_w: i32,
        src_h: i32,
    ) {
        self.draw_with(ctx, &self.up_shader.ps, src, dst, src_w, src_h);
    }

    /// 直接拷贝（尺寸一致）。
    fn draw_copy(
        &self,
        ctx: &ID3D11DeviceContext,
        src: &ID3D11ShaderResourceView,
        dst: &RenderTarget,
        src_w: i32,
        src_h: i32,
    ) {
        self.draw_with(ctx, &self.copy_shader.ps, src, dst, src_w, src_h);
    }

    /// 每采集帧跑一次。capture_srv 为全屏采集纹理。
    pub fn run(
        &self,
        ctx: &ID3D11DeviceContext,
        capture_srv: &ID3D11ShaderResourceView,
        w: i32,
        h: i32,
    ) {
        if self.down.len() != LEVELS || self.up.len() != LEVELS - 1 {
            return;
        }
        self.bind(ctx);
        unsafe {
            // Pass B leaves a per-block scissor active. Pass A is full-screen and
            // must reset it; otherwise only the last block rectangle is written and
            // the rest of every blur RT remains black.
            let full_scissor = windows::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: w,
                bottom: h,
            };
            ctx.RSSetScissorRects(Some(&[full_scissor]));
        }
        // --- 下采样 ---
        self.draw(ctx, capture_srv, &self.down[0], w, h);
        for k in 1..LEVELS {
            let src = self.down[k - 1].srv.clone();
            let (sw, sh) = (self.down[k - 1].w, self.down[k - 1].h);
            self.draw(ctx, &src, &self.down[k], sw, sh);
        }
        // --- 上采样（1/16 → 1/8 → 1/4 → 1/2） ---
        let mut src = self.down[LEVELS - 1].srv.clone();
        let mut src_w = self.down[LEVELS - 1].w;
        let mut src_h = self.down[LEVELS - 1].h;
        for k in (0..(LEVELS - 1)).rev() {
            let dst = &self.up[k];
            self.draw_up(ctx, &src, dst, src_w, src_h);
            src = dst.srv.clone();
            src_w = dst.w;
            src_h = dst.h;
        }

        // --- 块分辨率模糊链（避免 1/16 金字塔被放大的方块颗粒） ---
        if self.frost_half.len() == 2 {
            // 整屏 → 1/2；随后在同分辨率 ping-pong 4 次 tent，得到接近
            // D2D GaussianBlur sigma≈20 的连续模糊，但每帧仍只执行一套共享 Pass A。
            self.draw(ctx, capture_srv, &self.frost_half[0], w, h);
            // draw() leaves frost_half[0] bound as PS SRV. Unbind it before the
            // first ping-pong write; D3D11 otherwise silently nulls the RTV/SRV
            // hazard and preserves uninitialized black tiles.
            unsafe {
                let null = [None::<ID3D11ShaderResourceView>];
                ctx.PSSetShaderResources(0, Some(&null));
                ctx.OMSetRenderTargets(None, None);
            }
            for pass in 0..4 {
                let src_i = pass & 1;
                let dst_i = 1 - src_i;
                let src = self.frost_half[src_i].srv.clone();
                let (sw, sh) = (self.frost_half[src_i].w, self.frost_half[src_i].h);
                self.draw_up(ctx, &src, &self.frost_half[dst_i], sw, sh);
                unsafe {
                    let null = [None::<ID3D11ShaderResourceView>];
                    ctx.PSSetShaderResources(0, Some(&null));
                    ctx.OMSetRenderTargets(None, None);
                }
            }
            // 最终半分辨率结果上采样回整屏。
            let s1 = self.frost_half[0].srv.clone();
            let (sw, sh) = (self.frost_half[0].w, self.frost_half[0].h);
            if let Some(frost) = &self.frost {
                self.draw_up(ctx, &s1, frost, sw, sh);
            }
            if let Some(soft) = &self.soft {
                // 轻模糊：仅一次降采样后上采样，保留较多细节
                let (s0w, s0h) = (self.frost_half[0].w, self.frost_half[0].h);
                let s0b = self.frost_half[0].srv.clone();
                self.draw_up(ctx, &s0b, soft, s0w, s0h);
            }
        }
        // --- 环境色探针（1/16 分辨率，供自适应染色） ---
        if let Some(env) = &self.env16 {
            let s3 = self.down[LEVELS - 1].srv.clone();
            let (dw, dh) = (self.down[LEVELS - 1].w, self.down[LEVELS - 1].h);
            self.draw_copy(ctx, &s3, env, dw, dh);
        }
        unsafe {
            // Pass B binds t0..t8. All nine slots must be cleared before any
            // pyramid texture is rebound as an RTV on the next frame.
            let null: [Option<ID3D11ShaderResourceView>; 9] =
                [None, None, None, None, None, None, None, None, None];
            ctx.PSSetShaderResources(0, Some(&null));
            ctx.OMSetRenderTargets(None, None);
        }
    }
}
