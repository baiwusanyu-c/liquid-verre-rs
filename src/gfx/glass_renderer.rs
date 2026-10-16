//! Pass B：逐块玻璃绘制（方案 5.x / 7.1）。
//! 单画布 C 模式：所有块画进同一张全屏 RT，用 scissor 裁剪各自的矩形，
//! 一次 Present + 一次 DComp Commit —— 显存上界与块数无关。

use windows::Win32::Foundation::{BOOL, RECT as WinRect};
use windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY;
use windows::Win32::Graphics::Direct3D11::*;

use crate::gfx::pyramid::Pyramid;
use crate::gfx::shaders::{
    bytes, compile, make_dynamic_cbuffer, make_linear_sampler, make_premultiplied_blend,
    update_cbuffer,
};
use crate::glass::block::Block;
use crate::glass::params::Params;
use crate::glass::sdf::{build_refraction_lut, clamp_for_block, is_lod3, LUT_SIZE};

const TOPO_TRIANGLE: D3D_PRIMITIVE_TOPOLOGY = D3D_PRIMITIVE_TOPOLOGY(4);
const CB_FLOATS: usize = 18 * 4;

#[repr(C)]
#[derive(Clone, Copy)]
struct GlassCb {
    v: [f32; CB_FLOATS],
}

impl GlassCb {
    fn zeroed() -> Self {
        Self {
            v: [0.0; CB_FLOATS],
        }
    }
    fn set4(&mut self, slot: usize, x: f32, y: f32, z: f32, w: f32) {
        let i = slot * 4;
        self.v[i] = x;
        self.v[i + 1] = y;
        self.v[i + 2] = z;
        self.v[i + 3] = w;
    }
}

pub struct GlassRenderer {
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    cb: ID3D11Buffer,
    sampler: ID3D11SamplerState,
    blend: ID3D11BlendState,
    raster: ID3D11RasterizerState,
    depth: ID3D11DepthStencilState,
    lut: ID3D11Buffer,
    lut_srv: ID3D11ShaderResourceView,
    /// 环境色 EMA（方案 12.6）
    pub env_ema: [f32; 3],
    /// 本帧真正执行了 Draw 的块数（metrics 用）
    drawn: std::cell::Cell<usize>,
}

impl GlassRenderer {
    pub fn new(device: &ID3D11Device) -> Result<Self, String> {
        let vs_blob = compile(crate::gfx::shaders::GLASS_VS, "vs_main", "vs_5_0")?;
        let ps_blob = compile(crate::gfx::shaders::GLASS_PS, "ps_main", "ps_5_0")?;
        let vs = unsafe {
            let mut o = None;
            device
                .CreateVertexShader(bytes(&vs_blob), None, Some(&mut o))
                .map_err(|e| e.to_string())?;
            o.ok_or("CreateVertexShader(glass)")?
        };
        let ps = unsafe {
            let mut o = None;
            device
                .CreatePixelShader(bytes(&ps_blob), None, Some(&mut o))
                .map_err(|e| e.to_string())?;
            o.ok_or("CreatePixelShader(glass)")?
        };
        let cb = make_dynamic_cbuffer(device, (CB_FLOATS * 4) as u32)?;
        let sampler = make_linear_sampler(device)?;
        let blend = make_premultiplied_blend(device)?;

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
            o.ok_or("CreateRasterizerState(glass)")?
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
            o.ok_or("CreateDepthStencilState(glass)")?
        };

        // 结构化 LUT：256 项 float2（方案 5.4）
        let default_params = Params::default();
        let lut_data = build_refraction_lut(&default_params, Default::default());
        let sbd = D3D11_BUFFER_DESC {
            ByteWidth: (LUT_SIZE * 8) as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            MiscFlags: D3D11_RESOURCE_MISC_BUFFER_STRUCTURED.0 as u32,
            StructureByteStride: 8,
        };
        let mut sub = D3D11_SUBRESOURCE_DATA::default();
        sub.pSysMem = lut_data.as_ptr() as *const _;
        let mut lut = None;
        unsafe {
            device
                .CreateBuffer(&sbd, Some(&sub), Some(&mut lut))
                .map_err(|e| e.to_string())?
        };
        let lut = lut.ok_or("CreateBuffer(LUT)")?;
        let mut srv = None;
        unsafe {
            device
                .CreateShaderResourceView(&lut, None, Some(&mut srv))
                .map_err(|e| e.to_string())?;
        }
        let lut_srv = srv.ok_or("CreateShaderResourceView(LUT)")?;

        Ok(Self {
            vs,
            ps,
            cb,
            sampler,
            blend,
            raster,
            depth,
            lut,
            lut_srv,
            env_ema: [0.35, 0.42, 0.5],
            drawn: std::cell::Cell::new(0),
        })
    }

    /// Pass A 的清屏需要与 Pass B 相同的光栅化状态（开启 scissor）。
    pub fn raster_state(&self) -> &ID3D11RasterizerState {
        &self.raster
    }

    pub fn last_drawn(&self) -> usize {
        self.drawn.get()
    }

    pub fn update_lut(&self, ctx: &ID3D11DeviceContext, params: &Params) -> Result<(), String> {
        let data = build_refraction_lut(params, Default::default());
        let mut m = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            ctx.Map(&self.lut, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut m))
                .map_err(|e| format!("Map(LUT): {e}"))?;
            std::ptr::copy_nonoverlapping(
                data.as_ptr() as *const u8,
                m.pData as *mut u8,
                LUT_SIZE * 8,
            );
            ctx.Unmap(&self.lut, 0);
        }
        Ok(())
    }

    /// 绘制所有块（一次 RT 绑定，逐块 scissor + draw）
    #[allow(clippy::too_many_arguments)]
    pub fn draw_blocks(
        &self,
        ctx: &ID3D11DeviceContext,
        out_rtv: &ID3D11RenderTargetView,
        out_w: i32,
        out_h: i32,
        pyramid: &Pyramid,
        capture: &ID3D11ShaderResourceView,
        blocks: &[Block],
        order: &[usize],
        dirty: Option<[i32; 4]>,
        params: &Params,
        canvas_w: f32,
        canvas_h: f32,
        canvas_origin: [f32; 2],
        cursor_screen: [f32; 2],
        drag_velocity: [f32; 2],
        time: f32,
        press: f32,
        hover_id: Option<u32>,
    ) {
        self.drawn.set(0);
        let (Some(frost), Some(soft), Some(env16)) = (
            pyramid.frost.as_ref(),
            pyramid.soft.as_ref(),
            pyramid.env_rt(),
        ) else {
            return;
        };

        // 获取 pyramid 的 4 级模糊链用于 progressive blur
        let blur0 = pyramid.level(0).cloned();
        let blur1 = pyramid.level(1).cloned();
        let blur2 = pyramid.level(2).cloned();
        let blur3 = pyramid.level(3).cloned();
        let (Some(blur0), Some(blur1), Some(blur2), Some(blur3)) = (blur0, blur1, blur2, blur3)
        else {
            return;
        };

        unsafe {
            ctx.IASetInputLayout(None);
            ctx.IASetPrimitiveTopology(TOPO_TRIANGLE);
            ctx.OMSetRenderTargets(Some(&[Some(out_rtv.clone())]), None);
            let vp = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: out_w as f32,
                Height: out_h as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            ctx.RSSetViewports(Some(&[vp]));
            ctx.RSSetState(&self.raster);
            ctx.OMSetBlendState(&self.blend, Some(&[0.0; 4]), 0xffff_ffff);
            ctx.OMSetDepthStencilState(&self.depth, 0);
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShader(&self.ps, None);
            ctx.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
            ctx.VSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
            let srvs = [
                Some(capture.clone()),
                Some(frost.srv.clone()),
                Some(soft.srv.clone()),
                Some(env16.srv.clone()),
                Some(self.lut_srv.clone()),
                Some(blur0.srv.clone()),
                Some(blur1.srv.clone()),
                Some(blur2.srv.clone()),
                Some(blur3.srv.clone()),
            ];
            ctx.PSSetShaderResources(0, Some(&srvs));
        }

        for &index in order {
            let Some(block) = blocks.get(index) else {
                continue;
            };
            let mut p = params.clone();
            clamp_for_block(&mut p, block.width, block.height);
            p.corner_radius = block
                .radius
                .min(p.corner_radius)
                .min(0.5 * block.width.min(block.height));

            // 参考实现使用 12px 位移、40px 柔影；单画布中保留方案要求的
            // 24px 外扩，避免阴影被 scissor 截断，同时不创建每块 RT。
            let margin = p.shadow_margin;
            let rt_w = (block.width + margin * 2.0).ceil().max(1.0);
            let rt_h = (block.height + margin * 2.0).ceil().max(1.0);
            let rt_x = (block.x - margin).floor();
            let rt_y = (block.y - margin).floor();

            let raw_sx = rt_x.round() as i32;
            let raw_sy = rt_y.round() as i32;
            let raw_ex = raw_sx.saturating_add(rt_w.round() as i32);
            let raw_ey = raw_sy.saturating_add(rt_h.round() as i32);
            let sx = raw_sx.clamp(0, out_w);
            let sy = raw_sy.clamp(0, out_h);
            let ex = raw_ex.clamp(0, out_w);
            let ey = raw_ey.clamp(0, out_h);
            if ex <= sx || ey <= sy {
                continue;
            }
            // 脏矩形裁剪（方案 12.4.1 分级重绘）：与脏区不相交的块本帧完全跳过，
            // 相交的块把 scissor 收窄到交集，光栅化像素数只与脏区面积相关。
            let (mut fx0, mut fy0, mut fx1, mut fy1) = (sx, sy, ex, ey);
            if let Some(d) = dirty {
                fx0 = fx0.max(d[0]);
                fy0 = fy0.max(d[1]);
                fx1 = fx1.min(d[2]);
                fy1 = fy1.min(d[3]);
                if fx1 <= fx0 || fy1 <= fy0 {
                    continue;
                }
            }
            let scissor = WinRect {
                left: fx0,
                top: fy0,
                right: fx1,
                bottom: fy1,
            };
            unsafe {
                ctx.RSSetScissorRects(Some(&[scissor]));
                let block_vp = D3D11_VIEWPORT {
                    TopLeftX: 0.0,
                    TopLeftY: 0.0,
                    Width: out_w as f32,
                    Height: out_h as f32,
                    MinDepth: 0.0,
                    MaxDepth: 1.0,
                };
                ctx.RSSetViewports(Some(&[block_vp]));
            }

            let lod3 = if is_lod3(block.width, block.height) {
                1.0
            } else {
                0.0
            };
            let (shape_mode, super_n) = p.shape.uniform();
            let tt = time;
            let is_hover = hover_id == Some(block.id);
            let is_dragged = is_hover && press > 0.01;
            let hover = if is_hover { 1.0 } else { 0.0 };
            // Drag velocity is a screen-sampling compensation for the dragged
            // glass only. Applying it to every block makes all other glass
            // backgrounds visibly wobble when one block moves.
            let block_drag_velocity = if is_dragged {
                drag_velocity
            } else {
                [0.0, 0.0]
            };

            let mut cb = GlassCb::zeroed();
            // c0: 渲染目标原点(画布px) / 块窗口尺寸
            cb.set4(0, rt_x, rt_y, rt_w, rt_h);
            // c1: 显示器原点 / 显示器尺寸
            cb.set4(1, canvas_origin[0], canvas_origin[1], canvas_w, canvas_h);
            // c2: 时间 / dpiScale / aaWidth / shadowMargin
            cb.set4(2, tt, 1.0, 1.25, margin);
            // c3: superN / cornerRadius / lensCoverage / bevelWidthPx
            cb.set4(3, super_n, p.corner_radius, p.lens_coverage, p.bevel_width);
            // c4: ior / thickness / refrStrength / dispersion
            cb.set4(4, p.ior, p.thickness, p.refr_strength, p.dispersion);
            // c5: clarity / blurMax / blurMin / progressiveCurve
            // blurMax 直接进入 pyramid LOD，blurMin 保证边缘仍有可见 frost。
            cb.set4(5, p.clarity, p.blur_max, p.blur_min, p.progressive_curve);
            // c6: tintOpacity / adaptiveTint / baseAlpha / shadowOpacity
            cb.set4(
                6,
                p.tint_opacity,
                p.adaptive_tint,
                p.base_alpha,
                p.shadow_opacity,
            );
            // c7: tintColor rgba
            cb.set4(
                7,
                p.tint_color[0],
                p.tint_color[1],
                p.tint_color[2],
                p.tint_color[3],
            );
            // c8: 主光方向 xyz / 强度
            cb.set4(
                8,
                p.light_dir[0],
                p.light_dir[1],
                p.light_dir[2],
                p.light_intensity,
            );
            // c9: specIntensity / specPower / fresnelF0 / rimWidth
            cb.set4(9, p.spec_intensity, p.spec_power, p.fresnel_f0, p.rim_width);
            // c10: specSpread / edgeAlways / causticGain / absorption
            cb.set4(
                10,
                p.spec_spread,
                p.edge_always,
                p.caustic_gain,
                p.absorption,
            );
            // c11: mouse(screen px) xy / radius / intensity
            cb.set4(
                11,
                cursor_screen[0],
                cursor_screen[1],
                p.mouse_radius,
                p.mouse_intensity,
            );
            // c12: mouseSpread / hover / press / proximityGlow
            cb.set4(12, p.mouse_spread, hover, press, p.proximity_glow);
            // c13: dragVelocity xy / lagSeconds / pad
            cb.set4(
                13,
                block_drag_velocity[0],
                block_drag_velocity[1],
                p.lag_seconds,
                0.0,
            );
            // c14: mouseColor xyz / envTintMix
            cb.set4(
                14,
                p.mouse_color[0],
                p.mouse_color[1],
                p.mouse_color[2],
                p.env_tint_mix,
            );
            // c15: shapeMode / distScale / maxSlope / pad
            cb.set4(15, shape_mode, 1.0, crate::glass::params::MAX_SLOPE, 0.0);
            // c16: 块窗口左上角(画布px) xy / envEma / lod3
            cb.set4(16, rt_x, rt_y, self.env_ema[0], lod3);
            // c17: 画布屏幕原点 / 画布尺寸
            cb.set4(17, canvas_origin[0], canvas_origin[1], canvas_w, canvas_h);

            if update_cbuffer(ctx, &self.cb, &cb).is_err() {
                continue;
            }
            unsafe {
                ctx.Draw(6, 0);
            }
            self.drawn.set(self.drawn.get() + 1);
        }

        unsafe {
            let null: [Option<ID3D11ShaderResourceView>; 9] =
                [None, None, None, None, None, None, None, None, None];
            ctx.PSSetShaderResources(0, Some(&null));
            ctx.OMSetRenderTargets(None, None);
        }
    }
}
