//! Pass B：逐块玻璃绘制（方案 5.x / 7.1）。
//! 单画布 C 模式：所有块画进同一张全屏 RT，用 scissor 裁剪各自的矩形，
//! 一次 Present + 一次 DComp Commit —— 显存上界与块数无关。

use windows::Win32::Foundation::{BOOL, RECT as WinRect};
use windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY;
use windows::Win32::Graphics::Direct3D11::*;

use crate::core::draw::DrawBlock;
use crate::core::params::{LiquidParams, Profile};
use crate::core::sdf::{build_refraction_lut, clamp_for_block, is_lod3, LUT_SIZE};
use crate::error::Result;
use crate::gfx::pyramid::Pyramid;
use crate::gfx::shaders::{
    bytes, compile, make_dynamic_cbuffer, make_linear_sampler, make_premultiplied_blend,
    update_cbuffer,
};

const TOPO_TRIANGLE: D3D_PRIMITIVE_TOPOLOGY = D3D_PRIMITIVE_TOPOLOGY(4);
/// 常量缓冲区槽位数。
///
/// c0..c17 = 重构前的液态玻璃语义（**不可改动**，改了就和历史取证图不可比）；
/// c18 = (mode, opacity, -, -)                    透明玻璃模式（需求 F3）
/// 缩放反馈 border **不**占用这里的槽位：它用自己的 `BorderCb`
/// （`border_vs.hlsl` / `border_ps.hlsl`），见 `draw_border`。
/// 0.5.2 #1 起 border **内联**在 `draw_blocks` 的逐块循环里（画完第 i 块玻璃
/// 立刻画第 i 块 border），因此它受块间 z 序遮挡 —— 语义是「border 属于这一块
/// 视觉的一部分」，不是「盖在全部玻璃之上的叠加层」。
const CB_FLOATS: usize = 19 * 4;

/// BorderCb 的槽位数（b0..b6）。
const BORDER_CB_FLOATS: usize = 7 * 4;

const CONTENT_CB_FLOATS: usize = 3 * 4;

/// LUT 缓存条目。键 = `(ior 的位模式, profile)` —— LUT 只由这两项决定
/// （见 `core::sdf::build_refraction_lut`），因此同键必然同内容。
struct LutEntry {
    key: u64,
    /// 必须持有 buffer，否则 SRV 会指向已释放的资源。
    _buf: ID3D11Buffer,
    srv: ID3D11ShaderResourceView,
}

/// LUT 缓存上限。
///
/// 逐块 `ior` 可配之后理论上可以有任意多份 LUT，但一份只有 256x8 = 2 KB，
/// 8 份 = 16 KB；超出就 FIFO 淘汰。实际场景里同一应用的玻璃几乎共用一个 ior，
/// 因此命中率接近 100%，淘汰只是防御性上界。
const MAX_LUTS: usize = 8;

fn profile_index(profile: Profile) -> u8 {
    match profile {
        Profile::Circular => 0,
        Profile::Parabolic => 1,
        Profile::Sinusoidal => 2,
    }
}

/// LUT 缓存键。`ior` 用位模式，避免浮点相等比较的边界争论。
fn lut_key(p: &LiquidParams) -> u64 {
    (u64::from(p.ior.to_bits()) << 8) | u64::from(profile_index(p.profile))
}

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
    /// 默认参数（`LiquidParams::default()`）对应的 LUT，绘制前预绑定到 t4。
    lut_srv: ID3D11ShaderResourceView,
    /// 逐块 ior / profile 的 LUT 缓存（`draw_blocks` 取 `&self`，因此用内部可变性）。
    luts: std::cell::RefCell<Vec<LutEntry>>,
    /// 当前绑定到 t4 的 LUT 键；相同则跳过 `PSSetShaderResources`。
    bound_lut: std::cell::Cell<u64>,
    /// 缩放反馈 border 的独立管线（VS/PS/CB）。
    ///
    /// 单独一套而不是复用玻璃的 cbuffer：border 的矩形 / 掩码 / 颜色与玻璃参数
    /// 完全无关，混在一个 cbuffer 里会让「改玻璃参数」和「画 border」互相牵制。
    bvs: ID3D11VertexShader,
    bps: ID3D11PixelShader,
    bcb: ID3D11Buffer,
    cvs: ID3D11VertexShader,
    cps: ID3D11PixelShader,
    ccb: ID3D11Buffer,
    /// 环境色 EMA（方案 12.6）
    pub env_ema: [f32; 3],
    /// 本帧真正执行了 Draw 的块数（metrics 用）
    drawn: std::cell::Cell<usize>,
    /// border 发射计划的**复用缓冲**（每帧 clear + fill）。
    ///
    /// 为什么不是每帧 `Vec::new()`：`draw_blocks` 在热路径上，spec 明令
    /// 「热路径零堆分配」（20 块 × 120 Hz = 2400 次/秒的 malloc 就是回归）。
    /// 判据本体仍是同一个纯函数 `core::draw::border_emit_plan`，只是写进这里复用的缓冲。
    border_plan: std::cell::RefCell<Vec<(usize, usize)>>,
}

impl GlassRenderer {
    pub fn new(device: &ID3D11Device) -> crate::error::Result<Self> {
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
        let default_params = LiquidParams::default();
        let lut_data = build_refraction_lut(&default_params, default_params.profile);
        let sbd = D3D11_BUFFER_DESC {
            ByteWidth: (LUT_SIZE * 8) as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            MiscFlags: D3D11_RESOURCE_MISC_BUFFER_STRUCTURED.0 as u32,
            StructureByteStride: 8,
        };
        let sub = D3D11_SUBRESOURCE_DATA {
            pSysMem: lut_data.as_ptr() as *const _,
            ..Default::default()
        };
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

        // ---- border 叠加管线 ----
        let bvs_blob = compile(crate::gfx::shaders::BORDER_VS, "vs_main", "vs_5_0")?;
        let bps_blob = compile(crate::gfx::shaders::BORDER_PS, "ps_main", "ps_5_0")?;
        let bvs = unsafe {
            let mut o = None;
            device
                .CreateVertexShader(bytes(&bvs_blob), None, Some(&mut o))
                .map_err(|e| e.to_string())?;
            o.ok_or("CreateVertexShader(border)")?
        };
        let bps = unsafe {
            let mut o = None;
            device
                .CreatePixelShader(bytes(&bps_blob), None, Some(&mut o))
                .map_err(|e| e.to_string())?;
            o.ok_or("CreatePixelShader(border)")?
        };
        let bcb = make_dynamic_cbuffer(device, (BORDER_CB_FLOATS * 4) as u32)?;

        let cvs_blob = compile(crate::gfx::shaders::CONTENT_VS, "vs_main", "vs_5_0")?;
        let cps_blob = compile(crate::gfx::shaders::CONTENT_PS, "ps_main", "ps_5_0")?;
        let cvs = unsafe {
            let mut o = None;
            device
                .CreateVertexShader(bytes(&cvs_blob), None, Some(&mut o))
                .map_err(|e| e.to_string())?;
            o.ok_or("CreateVertexShader(content)")?
        };
        let cps = unsafe {
            let mut o = None;
            device
                .CreatePixelShader(bytes(&cps_blob), None, Some(&mut o))
                .map_err(|e| e.to_string())?;
            o.ok_or("CreatePixelShader(content)")?
        };
        let ccb = make_dynamic_cbuffer(device, (CONTENT_CB_FLOATS * 4) as u32)?;

        let default_key = lut_key(&default_params);
        Ok(Self {
            vs,
            ps,
            cb,
            sampler,
            blend,
            raster,
            depth,
            bvs,
            bps,
            bcb,
            cvs,
            cps,
            ccb,
            lut_srv,
            luts: std::cell::RefCell::new(Vec::new()),
            bound_lut: std::cell::Cell::new(default_key),
            env_ema: [0.35, 0.42, 0.5],
            drawn: std::cell::Cell::new(0),
            border_plan: std::cell::RefCell::new(Vec::new()),
        })
    }

    /// 新建一份结构化 LUT（256 x float2）。
    fn create_lut(
        device: &ID3D11Device,
        data: &[[f32; 2]],
    ) -> Result<(ID3D11Buffer, ID3D11ShaderResourceView)> {
        let sbd = D3D11_BUFFER_DESC {
            ByteWidth: (LUT_SIZE * 8) as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            MiscFlags: D3D11_RESOURCE_MISC_BUFFER_STRUCTURED.0 as u32,
            StructureByteStride: 8,
        };
        let sub = D3D11_SUBRESOURCE_DATA {
            pSysMem: data.as_ptr() as *const _,
            ..Default::default()
        };
        let mut buf = None;
        unsafe {
            device
                .CreateBuffer(&sbd, Some(&sub), Some(&mut buf))
                .map_err(|e| format!("CreateBuffer(LUT): {e}"))?
        };
        let buf = buf.ok_or("CreateBuffer(LUT) 返回空")?;
        let mut srv = None;
        unsafe {
            device
                .CreateShaderResourceView(&buf, None, Some(&mut srv))
                .map_err(|e| format!("CreateShaderResourceView(LUT): {e}"))?;
        }
        let srv = srv.ok_or("CreateShaderResourceView(LUT) 返回空")?;
        Ok((buf, srv))
    }

    /// 绑定该块 ior / profile 对应的 LUT 到 t4。
    ///
    /// 键相同则**一次 API 调用都不发**（连续同 ior 的块是常态），
    /// 因此「逐块 ior 可配」在默认场景下的额外成本是每块一次整数比较。
    fn bind_lut(
        &self,
        device: &ID3D11Device,
        ctx: &ID3D11DeviceContext,
        p: &LiquidParams,
    ) -> Result<()> {
        let key = lut_key(p);
        if self.bound_lut.get() == key {
            return Ok(());
        }
        if let Some(slot) = self.luts.borrow().iter().position(|e| e.key == key) {
            let srv = self.luts.borrow()[slot].srv.clone();
            unsafe {
                ctx.PSSetShaderResources(4, Some(&[Some(srv)]));
            }
            self.bound_lut.set(key);
            return Ok(());
        }
        let data = build_refraction_lut(p, p.profile);
        let (buf, srv) = Self::create_lut(device, &data)?;
        {
            let mut luts = self.luts.borrow_mut();
            if luts.len() >= MAX_LUTS {
                // FIFO 淘汰最旧的一份；正在使用的绑定会在下一个块重新设置。
                luts.remove(0);
            }
            luts.push(LutEntry {
                key,
                _buf: buf,
                srv: srv.clone(),
            });
        }
        unsafe {
            ctx.PSSetShaderResources(4, Some(&[Some(srv)]));
        }
        self.bound_lut.set(key);
        Ok(())
    }

    /// Pass A 的清屏需要与 Pass B 相同的光栅化状态（开启 scissor）。
    pub fn raster_state(&self) -> &ID3D11RasterizerState {
        &self.raster
    }

    pub fn last_drawn(&self) -> usize {
        self.drawn.get()
    }

    /// 绘制所有块（一次 RT 绑定，逐块 scissor + draw）。
    ///
    /// 每块自带 `params`（已按 DPI 缩放 + 自适应降档），因此这里不再接收全局参数；
    /// `clamp_for_block`（方案 12.9 的逐块尺寸夹紧）仍在本函数内做，因为它依赖
    /// 块的**实际物理尺寸**，只有这里拿得到。
    #[allow(clippy::too_many_arguments)]
    pub fn draw_blocks(
        &self,
        device: &ID3D11Device,
        ctx: &ID3D11DeviceContext,
        out_rtv: &ID3D11RenderTargetView,
        out_w: i32,
        out_h: i32,
        pyramid: &Pyramid,
        capture: &ID3D11ShaderResourceView,
        blocks: &[DrawBlock],
        order: &[usize],
        canvas_w: f32,
        canvas_h: f32,
        canvas_origin: [f32; 2],
        cursor_screen: [f32; 2],
        drag_velocity: [f32; 2],
        time: f32,
        press: f32,
        hover_id: Option<u32>,
        contents: Option<&crate::gfx::content_layer::ContentStore>,
        border: Option<&BorderTarget<'_>>,
    ) {
        self.drawn.set(0);
        // t4 在下面的 PSSetShaderResources(0..9) 里被重置成默认 LUT，
        // 因此「当前绑定键」必须同步复位，否则第一个块会误判成已绑定而跳过。
        self.bound_lut.set(lut_key(&LiquidParams::default()));
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
        // 内容层（P0-1）：只有 store 里真的有层时才多绑一个 CB 槽位（b1）；
        // 空 store => 本帧与重构前逐位一致，零新增 API 调用（FR5 / 第一门禁前提）。
        let store = contents.filter(|s| !s.is_empty());
        let full_vp = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: out_w as f32,
            Height: out_h as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        let default_lut_key = lut_key(&LiquidParams::default());
        // 0.5.2 #1：border 的发射计划。判据本体是纯函数
        // `core::draw::border_emit_plan`（无 GPU 单测覆盖），这里只负责问「哪块有
        // feedback」。没有 feedback 块 => 空计划 => 本帧一次 border 状态切换都不发生。
        // 写进复用缓冲（`self.border_plan`）而不是每帧新建 Vec：热路径零堆分配。
        {
            let mut plan = self.border_plan.borrow_mut();
            plan.clear();
            if border.is_some() {
                crate::core::draw::border_emit_plan(
                    order,
                    |idx| blocks.get(idx).is_some_and(|b| !b.feedback.is_empty()),
                    &mut plan,
                );
            }
        }
        let plan = self.border_plan.borrow();

        for (seq, &index) in order.iter().enumerate() {
            let Some(block) = blocks.get(index) else {
                continue;
            };
            // ---- 每块开头无条件重设全套玻璃管线状态（0.5.2 #1 的结构性护栏）----
            // 为什么不再「循环外设一次」：border 现在内联在循环里，上一块可能把管线
            // 留在 border（bvs/bps/bcb）或内容（cvs/cps/ccb + t9）上。「谁改了什么
            // 就还原什么」这张清单历史上正是漏了一处（content pass 后不整体重绑
            // t0..t8）导致下一块玻璃整块消失（gotchas #3，2026-10-26 bring-up 实测）。
            // 逐块钉死状态把这一整类错误变成不可能，代价是每帧多 ~13 次 Set* 调用
            // × 块数；D3D11 的同值 Set* 早退很快，实测 20 块 Pass B 仍在 0.1 ms 量级
            // （性能阀门 tools/gate_perf.py 会盯住它）。
            unsafe {
                ctx.IASetInputLayout(None);
                ctx.IASetPrimitiveTopology(TOPO_TRIANGLE);
                ctx.OMSetRenderTargets(Some(&[Some(out_rtv.clone())]), None);
                ctx.RSSetViewports(Some(&[full_vp]));
                ctx.RSSetState(&self.raster);
                ctx.OMSetBlendState(&self.blend, Some(&[0.0; 4]), 0xffff_ffff);
                ctx.OMSetDepthStencilState(&self.depth, 0);
                ctx.VSSetShader(&self.vs, None);
                ctx.PSSetShader(&self.ps, None);
                ctx.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
                ctx.PSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
                ctx.VSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
                // t4 跟着整体重绑成默认 LUT，所以 bound_lut 必须同步复位，
                // 否则 bind_lut 会拿上一块的键跳过重绑、采到错的 LUT。
                ctx.PSSetShaderResources(0, Some(&srvs));
                if store.is_some() {
                    ctx.VSSetConstantBuffers(1, Some(&[Some(self.ccb.clone())]));
                    ctx.PSSetConstantBuffers(1, Some(&[Some(self.ccb.clone())]));
                }
            }
            self.bound_lut.set(default_lut_key);
            let mut p = block.params;
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
            // 这里**刻意不做**「与脏区不相交的块跳过 / scissor 收窄到交集」：
            // flip 模型的 back buffer 在 Present 后不保内容（不变量 1），只画脏块
            // 会让没画的块整块消失 = 用户看到的闪动。这个参数历史上唯一调用点
            // 传的字面量 `None`，分支是死码（独立审查 OVERDESIGN #2），删掉。
            // 脏区的价值体现在 Pass A 的 scissor 与「这一帧值不值得渲染」的门控上。
            let (fx0, fy0, fx1, fy1) = (sx, sy, ex, ey);
            let scissor = WinRect {
                left: fx0,
                top: fy0,
                right: fx1,
                bottom: fy1,
            };
            unsafe {
                // viewport 已由本块开头的统一重置给出（整画布），这里只切 scissor。
                ctx.RSSetScissorRects(Some(&[scissor]));
            }

            let lod3 = if is_lod3(block.width, block.height) {
                1.0
            } else {
                0.0
            };
            let (shape_mode, shape_super_n) = p.shape.uniform();
            // super_n 现在真的可配（需求 F1）：Squircle 用上层给的指数，
            // RoundedRect 分支不读它，因此默认（RoundedRect）下与重构前逐位一致。
            let super_n = if shape_mode > 0.5 {
                p.super_n.max(2.0)
            } else {
                shape_super_n
            };
            // 透明模式（需求 F3）：着色器按 c18.x 走早退分支，
            // 不采样 capture / frost / env / LUT。
            let mode = if block.mode.needs_backdrop() {
                0.0
            } else {
                1.0
            };
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
            cb.set4(15, shape_mode, 1.0, crate::core::params::MAX_SLOPE, 0.0);
            // c16: 块窗口左上角(画布px) xy / envEma / lod3
            cb.set4(16, rt_x, rt_y, self.env_ema[0], lod3);
            // c17: 画布屏幕原点 / 画布尺寸
            cb.set4(17, canvas_origin[0], canvas_origin[1], canvas_w, canvas_h);
            // c18: mode / 透明模式不透明度（= base_alpha，已含颜色 alpha）
            // c18.z = 「缩放把手被抓住」：border 由独立 pass 画，但液态着色器原有的
            // hover 高光需要让位（见 glass_ps.hlsl 的 noFb），否则直边会残留白线。
            let grabbed = if block.feedback.is_empty() { 0.0 } else { 1.0 };
            cb.set4(18, mode, p.base_alpha, grabbed, 0.0);

            // 逐块 LUT：ior / profile 相同的连续块只发一次绑定。
            if self.bind_lut(device, ctx, &p).is_err() {
                continue;
            }
            if update_cbuffer(ctx, &self.cb, &cb).is_err() {
                continue;
            }
            unsafe {
                ctx.Draw(6, 0);
            }
            self.drawn.set(self.drawn.get() + 1);
            // 内容必须**在本块玻璃之后、下一块玻璃之前**画（FR3）。
            // 为什么不是「全部玻璃画完再统一画内容」的批量 pass：那样下层块的内容会
            // 浮在上层块玻璃之上。具体实例：两块 50% 重叠（下块先画），批量结构下重叠区
            // 采到的像素 = 裸 checker 纹素（内容覆盖了上层玻璃的混色结果）；交错结构下
            // = 上层玻璃 premul 混在内容上。verify_content.py 断言 2 就是拿这个对照
            // 数值做的硬判据（2026-10-26 实测数据见该脚本头部注释）。
            // 无内容的块在这里只付一次 HashMap 查找（且仅当 store 非空）；空 store
            // 时 store=None，连查找都没有。
            if let Some(store) = store {
                if let Some(layer) = store.get(block.id) {
                    self.draw_content_for_block(
                        ctx,
                        block,
                        p.corner_radius,
                        layer,
                        canvas_w,
                        canvas_h,
                        out_w,
                        out_h,
                        &srvs,
                    );
                }
            }
            // ---- 0.5.2 #1：本块的 border 紧跟本块玻璃（及本块内容）之后 ----
            // 于是「比它高的块的玻璃」一定画在它之后 = 把它盖住。border 是这一块
            // 视觉的一部分，不是全局叠加层。`seq` 用玻璃序而不是块下标：order 允许
            // 是 blocks 的任意排列（z 序排序在投影层做）。
            if let Some(out) = border {
                if plan.iter().any(|&(_, s)| s == seq) {
                    self.draw_border(ctx, out, block);
                }
            }
        }

        unsafe {
            let null: [Option<ID3D11ShaderResourceView>; 9] =
                [None, None, None, None, None, None, None, None, None];
            ctx.PSSetShaderResources(0, Some(&null));
            if store.is_some() {
                // t9 只在有内容时被写过；解绑防 hazard（gotchas #3）。
                let null9: [Option<ID3D11ShaderResourceView>; 1] = [None];
                ctx.PSSetShaderResources(9, Some(&null9));
            }
            ctx.OMSetRenderTargets(None, None);
        }
    }

    /// 画单块内容（紧接该块玻璃的 Draw 之后）。
    ///
    /// 与玻璃 pass 共享的东西**都不重绑**：RT、viewport（整画布）、raster（scissor 开）、
    /// premul blend、depth、CB 槽位 0（玻璃 cbuffer）、t0..t8（玻璃纹理）、s0（linear
    /// clamp）。内容只用自己的 b1 cbuffer 与 t9 纹理槽位——PS 用整数 `Load` 取点，
    /// 不经过采样器，所以连 sampler 都不占。切换成本 = 2 次 SetShader + 1 次 SetSRV +
    /// 1 次 CB 更新 + 1 次 scissor + 1 次 Draw，且只对**有内容的块**发生。
    #[allow(clippy::too_many_arguments)]
    fn draw_content_for_block(
        &self,
        ctx: &ID3D11DeviceContext,
        block: &DrawBlock,
        radius: f32,
        layer: &crate::gfx::content_layer::ContentLayer,
        canvas_w: f32,
        canvas_h: f32,
        out_w: i32,
        out_h: i32,
        glass_srvs: &[Option<ID3D11ShaderResourceView>; 9],
    ) {
        // 纹理锚定块矩形左上角 1:1；锚点取整保证纹素边界与设备像素对齐
        // （取证断言 1 的「精确 checker 值」依赖这一点）。scissor 用同一取整矩形。
        let rx0 = block.x.round() as i32;
        let ry0 = block.y.round() as i32;
        let rw = (block.width.round() as i32).max(1);
        let rh = (block.height.round() as i32).max(1);
        let sx = rx0.max(0);
        let sy = ry0.max(0);
        let ex = rx0.saturating_add(rw).max(0);
        let ey = ry0.saturating_add(rh).max(0);
        let mut cb = ContentCb {
            v: [0.0; CONTENT_CB_FLOATS],
        };
        let set4 = |cb: &mut ContentCb, slot: usize, a: f32, b: f32, c: f32, d: f32| {
            let i = slot * 4;
            cb.v[i] = a;
            cb.v[i + 1] = b;
            cb.v[i + 2] = c;
            cb.v[i + 3] = d;
        };
        set4(&mut cb, 0, rx0 as f32, ry0 as f32, rw as f32, rh as f32);
        // c1.z = 该块本次绘制用的**最终** corner radius（与玻璃 c3.y 同一个
        // p.corner_radius，C5 单一来源——不是第二处半径推导）。
        set4(&mut cb, 1, layer.w as f32, layer.h as f32, radius, 1.25);
        set4(&mut cb, 2, canvas_w, canvas_h, 0.0, 0.0);
        if update_cbuffer(ctx, &self.ccb, &cb).is_err() {
            return;
        }
        unsafe {
            ctx.PSSetShaderResources(9, Some(&[Some(layer.srv.clone())]));
            let cx1 = ex.min(out_w);
            let cy1 = ey.min(out_h);
            ctx.RSSetScissorRects(Some(&[WinRect {
                left: sx,
                top: sy,
                right: cx1,
                bottom: cy1,
            }]));
            ctx.VSSetShader(&self.cvs, None);
            ctx.PSSetShader(&self.cps, None);
            ctx.Draw(6, 0);
            // 切回玻璃管线。t9 的 content SRV 绝不能活到下一块玻璃：bind_lut 在
            // 逐块 LUT 切换时只重绑 t4 单槽，若 content pass 之后不整体重绑
            // t0..t8，玻璃采样与绘制会踩 gotchas #3 的静默丢弃（2026-10-26
            // bring-up 实测：content pass 后下一块玻璃整块消失，红屏实验定位）。
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShader(&self.ps, None);
            ctx.PSSetShaderResources(0, Some(glass_srvs));
            let null9: [Option<ID3D11ShaderResourceView>; 1] = [None];
            ctx.PSSetShaderResources(9, Some(&null9));
        }
        // 0.5.2 附带修正（与 border 内联同一处结构性风险）：上面整体重绑 t0..t8
        // 时，t4 被打回**默认** LUT，而 `bound_lut` 还记着上一块的自定义键 ——
        // 于是下一块若同键就跳过重绑，采到默认 LUT（折射曲线错、不报错）。
        // 账本必须跟着实际绑定走。
        self.bound_lut.set(lut_key(&LiquidParams::default()));
    }
}

/// `draw_border` 的渲染目标描述（0.5.2 #1：border 由 `draw_blocks` 逐块内联发射）。
pub(crate) struct BorderTarget<'a> {
    /// 画布后备缓冲 RTV。
    pub(crate) rtv: &'a ID3D11RenderTargetView,
    /// 画布宽（物理像素）。
    pub(crate) w: i32,
    /// 画布高（物理像素）。
    pub(crate) h: i32,
    /// 画布尺寸（浮点，VS 里做 NDC 换算用）。
    pub(crate) canvas: [f32; 2],
}

/// BorderCb 的 Rust 镜像（b0..b6，与 border_vs/border_ps 的 cbuffer 逐字段对应）。
#[repr(C)]
#[derive(Clone, Copy)]
struct BorderCb {
    v: [f32; BORDER_CB_FLOATS],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ContentCb {
    v: [f32; CONTENT_CB_FLOATS],
}

impl GlassRenderer {
    /// 画**单块**缩放反馈 border（需求 F2；0.5.2 #1 起由 `draw_blocks` 逐块内联调用）。
    ///
    /// 返回 `false` = 该块没有 feedback（或画布内无有效像素），调用方零成本跳过，
    /// 一个状态切换都不发。
    ///
    /// 与玻璃共用 premultiplied blend / scissor 光栅化状态，因此叠上去的语义和
    /// 「玻璃自己画出来的白色」完全一致。但它**必须**紧跟自己那块的玻璃画完：
    /// 比它高的块的玻璃随后会把它盖住 —— 这就是 0.5.2 #1 的修复点（旧实现是
    /// 「全部玻璃画完后的独立叠加 pass」，两块分类框重叠时下层框的白边会画到
    /// 上层框身上）。⚠ 这只修了**绘制**层序；**命中**层序（把手让位更上层块本体）
    /// 是 0.5.3 #9 的另一半，见 BASELINE 不变量 13 —— 别以为抬层能绕。
    ///
    /// ⚠ 三条不能回退的实现要点（清单里逐条点名的「决定成败」项）：
    /// 1. **绝对不要在尾部 `OMSetRenderTargets(None, None)`** —— 旧 `draw_borders`
    ///    尾部那两行是「独立 pass 收尾」，内联版之后还要往同一张 RT 画更高块的
    ///    玻璃，解绑 = 下一块整块画不出来 = 闪黑。
    /// 2. scissor 口径保持不变：CB 用**未 clamp** 的 `draw_rect()`（玻璃 c0/c16
    ///    同样不 clamp，两个 pass 的 local 原点必须同源，否则块半出屏时白框整体
    ///    错位 —— 独立审查 P1-2），clamp 只属于 scissor。
    /// 3. 这里重设**全套** border 管线状态，而不是「只改差异项」：`draw_blocks`
    ///    每块开头同样无条件重设玻璃状态，两边都不需要还原代码，也就不存在
    ///    「漏还原某一项」这类错误（gotchas #3 的前科正是漏还原 t0..t8）。
    pub(crate) fn draw_border(
        &self,
        ctx: &ID3D11DeviceContext,
        out: &BorderTarget<'_>,
        block: &DrawBlock,
    ) -> bool {
        let fb = block.feedback;
        if fb.is_empty() {
            return false;
        }
        let (out_rtv, out_w, out_h, canvas_w, canvas_h) =
            (out.rtv, out.w, out.h, out.canvas[0], out.canvas[1]);
        let [rx0, ry0, rx1, ry1] = block.draw_rect();
        let sx = rx0.max(0).min(out_w);
        let sy = ry0.max(0).min(out_h);
        let ex = rx1.max(0).min(out_w);
        let ey = ry1.max(0).min(out_h);
        if ex <= sx || ey <= sy {
            return false;
        }
        unsafe {
            ctx.IASetInputLayout(None);
            ctx.IASetPrimitiveTopology(TOPO_TRIANGLE);
            ctx.OMSetRenderTargets(Some(&[Some(out_rtv.clone())]), None);
            ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: out_w as f32,
                Height: out_h as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            ctx.RSSetState(&self.raster);
            ctx.OMSetBlendState(&self.blend, Some(&[0.0; 4]), 0xffff_ffff);
            ctx.OMSetDepthStencilState(&self.depth, 0);
            ctx.VSSetShader(&self.bvs, None);
            ctx.PSSetShader(&self.bps, None);
            ctx.VSSetConstantBuffers(0, Some(&[Some(self.bcb.clone())]));
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.bcb.clone())]));
        }
        let mut cb = BorderCb {
            v: [0.0; BORDER_CB_FLOATS],
        };
        let set4 = |cb: &mut BorderCb, slot: usize, a: f32, b: f32, c: f32, d: f32| {
            let i = slot * 4;
            cb.v[i] = a;
            cb.v[i + 1] = b;
            cb.v[i + 2] = c;
            cb.v[i + 3] = d;
        };
        set4(
            &mut cb,
            0,
            rx0 as f32,
            ry0 as f32,
            (rx1 - rx0) as f32,
            (ry1 - ry0) as f32,
        );
        set4(
            &mut cb,
            1,
            block.width * 0.5,
            block.height * 0.5,
            block.radius,
            1.25,
        );
        set4(
            &mut cb,
            2,
            fb.edges[0],
            fb.edges[1],
            fb.edges[2],
            fb.edges[3],
        );
        set4(
            &mut cb,
            3,
            fb.corners[0],
            fb.corners[1],
            fb.corners[2],
            fb.corners[3],
        );
        set4(&mut cb, 4, fb.width, fb.alpha, 0.0, 0.0);
        set4(&mut cb, 5, fb.color[0], fb.color[1], fb.color[2], 0.0);
        set4(&mut cb, 6, canvas_w, canvas_h, 0.0, 0.0);
        if update_cbuffer(ctx, &self.bcb, &cb).is_err() {
            return false;
        }
        unsafe {
            ctx.RSSetScissorRects(Some(&[WinRect {
                left: sx,
                top: sy,
                right: ex,
                bottom: ey,
            }]));
            ctx.Draw(6, 0);
        }
        true
    }
}
