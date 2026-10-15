//! Pass A：全屏共享模糊金字塔（方案 5.2）。
//!
//!   gBlur0 = 1/2   gBlur1 = 1/4   gBlur2 = 1/8   gBlur3 = 1/16
//!   下采样：capture -> 1/2 -> 1/4 -> 1/8 -> 1/16
//!
//! 相对旧实现移除的三处纯浪费（逐像素输出完全一致，视觉零变化）：
//!   1. `up` 上采样链：`glass_ps.hlsl` 里 `SamplePyramid` 从未被 `ps_main` 调用，
//!      gBlur0..3 不会被采样，因此 1/16 -> 1/8 -> 1/4 -> 1/2 的三次 tent 重建
//!      是死工作（约 0.33 个全屏当量 + 8.5 MB 常驻）。
//!   2. `soft` 独立 RT：旧代码用同一张源、同一个 tent pass 生成 `frost` 与
//!      `soft`，两者逐像素相同；现在共享同一份 RT/SRV（省 1 张整屏 RT + 1 次
//!      整屏 pass，3120x2080 下约 26 MB）。
//!   3. `env16` 拷贝：旧代码把 down[3] 原样再画一遍到同尺寸 RT；现在直接复用
//!      down[3]（归一化 UV 采样，结果完全一致）。
//! 同时去掉每个金字塔步骤里多余的 ClearRenderTargetView —— Pass A 用的是不透明
//! 混合（BlendEnable=0）+ 覆盖全视口的三角形，目标会被逐像素覆写。

use windows::Win32::Foundation::RECT;
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
    /// 强模糊 ping-pong 链：两个 1/2 分辨率 RT，重复 tent pass 近似
    /// liquid-glass-rs 的 D2D GaussianBlur sigma≈20，同时仍为全屏共享资源。
    frost_half: Vec<RenderTarget>,
    /// 块分辨率的强模糊（整屏尺寸，Pass B 直接采样）
    pub frost: Option<RenderTarget>,
    /// 轻模糊层。与 `frost` 共享同一份资源（见文件头说明）。
    pub soft: Option<RenderTarget>,
    /// 1/16 环境色探针，直接指向 down[3]
    pub env16: Option<RenderTarget>,
    down_shader: SimpleShader,
    up_shader: SimpleShader,
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
        let cb = make_dynamic_cbuffer(device, std::mem::size_of::<BlurCb>() as u32)?;
        let sampler = make_linear_sampler(device)?;
        let states = make_opaque_states(device)?;
        Ok(Self {
            down: Vec::new(),
            frost_half: Vec::new(),
            frost: None,
            soft: None,
            env16: None,
            down_shader,
            up_shader,
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
    pub fn level(&self, i: usize) -> Option<&RenderTarget> {
        self.down.get(i)
    }

    pub fn resize(&mut self, device: &ID3D11Device, w: i32, h: i32) -> Result<(), String> {
        let (w, h) = (w.max(4), h.max(4));
        if self.width == w && self.height == h && self.down.len() == LEVELS {
            return Ok(());
        }
        self.width = w;
        self.height = h;
        self.down.clear();
        for k in 0..LEVELS {
            let (dw, dh) = ((w >> (k + 1)).max(1), (h >> (k + 1)).max(1));
            self.down
                .push(make_rt(device, dw, dh, DXGI_FORMAT_B8G8R8A8_UNORM)?);
        }
        // 块分辨率模糊链：整屏 -> 1/2 -> 1/2（tent x4，等效 σ≈20px）-> 上采样回整屏
        self.frost_half.clear();
        for _ in 0..2 {
            let (hw, hh) = ((w >> 1).max(1), (h >> 1).max(1));
            self.frost_half
                .push(make_rt(device, hw, hh, DXGI_FORMAT_B8G8R8A8_UNORM)?);
        }
        let frost = make_rt(device, w, h, DXGI_FORMAT_B8G8R8A8_UNORM)?;
        // soft 与 frost 内容一致，共享同一份资源
        self.soft = Some(frost.clone());
        self.frost = Some(frost);
        // 环境色探针直接复用 1/16 层
        self.env16 = self.down.get(LEVELS - 1).cloned();
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
        scissor: RECT,
    ) {
        self.draw_with(ctx, &self.down_shader.ps, src, dst, src_w, src_h, scissor);
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
        scissor: RECT,
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
            ctx.RSSetScissorRects(Some(&[scissor]));
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
        scissor: RECT,
    ) {
        self.draw_with(ctx, &self.up_shader.ps, src, dst, src_w, src_h, scissor);
    }

    /// 把「全屏坐标系」里的裁剪矩形换算到某个金字塔层自己的分辨率。
    fn scissor_for(&self, rect: [i32; 4], dst: &RenderTarget) -> RECT {
        let sx = dst.w as f32 / self.width.max(1) as f32;
        let sy = dst.h as f32 / self.height.max(1) as f32;
        let l = ((rect[0] as f32 * sx).floor() as i32).clamp(0, dst.w);
        let t = ((rect[1] as f32 * sy).floor() as i32).clamp(0, dst.h);
        let r = ((rect[2] as f32 * sx).ceil() as i32).clamp(0, dst.w);
        let b = ((rect[3] as f32 * sy).ceil() as i32).clamp(0, dst.h);
        RECT {
            left: l,
            top: t,
            right: r.max(l + 1),
            bottom: b.max(t + 1),
        }
    }

    /// 每采集帧跑一次。capture_srv 为全屏采集纹理。
    ///
    /// `region` 是「全屏坐标系」下唯一需要刷新的矩形（块足迹 ⊕ 采样余量）。
    /// 金字塔是全屏共享资源，但块只会采样自己足迹附近的像素，因此把每一步
    /// 都用 scissor 收窄到该区域即可 —— 区域外保留上一帧内容，永远不会被采样。
    /// 这一步把 Pass A 的成本从 O(屏幕) 降到 O(块足迹)，是方案 5.2/7.4-2 的落地。
    pub fn run(
        &self,
        ctx: &ID3D11DeviceContext,
        capture_srv: &ID3D11ShaderResourceView,
        w: i32,
        h: i32,
        region: [i32; 4],
    ) {
        if self.down.len() != LEVELS {
            return;
        }
        let region = [
            region[0].clamp(0, w),
            region[1].clamp(0, h),
            region[2].clamp(0, w),
            region[3].clamp(0, h),
        ];
        if region[2] <= region[0] || region[3] <= region[1] {
            return;
        }
        self.bind(ctx);
        // --- 下采样 ---
        {
            let sc = self.scissor_for(region, &self.down[0]);
            self.draw(ctx, capture_srv, &self.down[0], w, h, sc);
        }
        for k in 1..LEVELS {
            let src = self.down[k - 1].srv.clone();
            let (sw, sh) = (self.down[k - 1].w, self.down[k - 1].h);
            let sc = self.scissor_for(region, &self.down[k]);
            self.draw(ctx, &src, &self.down[k], sw, sh, sc);
        }

        // --- 块分辨率模糊链（避免 1/16 金字塔被放大的方块颗粒） ---
        if self.frost_half.len() == 2 {
            if let Some(frost) = &self.frost {
                let sc_half = self.scissor_for(region, &self.frost_half[0]);
                // 整屏 -> 1/2；随后在同分辨率 ping-pong 4 次 tent，得到接近
                // D2D GaussianBlur sigma≈20 的连续模糊，但每帧仍只执行一套共享 Pass A。
                self.draw(ctx, capture_srv, &self.frost_half[0], w, h, sc_half);
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
                    let sc = self.scissor_for(region, &self.frost_half[dst_i]);
                    self.draw_up(ctx, &src, &self.frost_half[dst_i], sw, sh, sc);
                    unsafe {
                        let null = [None::<ID3D11ShaderResourceView>];
                        ctx.PSSetShaderResources(0, Some(&null));
                        ctx.OMSetRenderTargets(None, None);
                    }
                }
                // 最终半分辨率结果上采样回整屏。soft 与 frost 共享，只需一次。
                let s1 = self.frost_half[0].srv.clone();
                let (sw, sh) = (self.frost_half[0].w, self.frost_half[0].h);
                let sc_full = self.scissor_for(region, frost);
                self.draw_up(ctx, &s1, frost, sw, sh, sc_full);
            }
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
