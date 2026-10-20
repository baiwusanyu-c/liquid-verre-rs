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
    /// 1/16 环境色探针的时域平滑 ping-pong（方案 12.6 / R-20）
    env_rt: Vec<RenderTarget>,
    /// 最新 EMA 结果所在的下标；-1 表示还没有历史
    env_latest: std::cell::Cell<i8>,
    /// 上次 EMA 覆盖的全屏坐标矩形，用于判断是否需要重置历史
    env_region: std::cell::Cell<[i32; 4]>,
    /// 上次跑 EMA 的时刻，用于按真实 dt 计算 alpha
    env_last: std::cell::Cell<std::time::Instant>,
    env_enabled: bool,
    env_tau: f32,
    /// `GLASS_FROST_HALF=1`：`frost`/`soft` 直接指向 1/2 分辨率的模糊结果，
    /// 省掉 Pass A 里那次整屏 8-tap tent 上采样（约 1.0 个整屏当量 + 26 MB 常驻）。
    /// Pass B 用同一个归一化 UV 采样，放大交给双线性采样器。
    ///
    /// 默认关闭。**开启后输出不再与默认档逐位一致** —— 模糊层本来就是重模糊图
    /// （等效 σ≈20 px），差异预计很小，但属于视觉改动，需要显式开启并自行取证。
    frost_half_mode: bool,
    /// 冻结 EMA 推进：自证模式下同一帧要连渲两次，第二次必须复用完全相同的
    /// 环境色纹理，否则 dt 不同会让 alpha 不同，污染逐像素对照结果。
    env_freeze: std::cell::Cell<bool>,
    down_shader: SimpleShader,
    up_shader: SimpleShader,
    env_shader: SimpleShader,
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
        let env_shader = make_shader_pair(
            device,
            crate::gfx::shaders::BLUR_VS,
            "vs_main",
            crate::gfx::shaders::ENV_EMA,
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
            env_rt: Vec::new(),
            env_latest: std::cell::Cell::new(-1),
            env_region: std::cell::Cell::new([0, 0, 0, 0]),
            env_last: std::cell::Cell::new(std::time::Instant::now()),
            env_enabled: std::env::var("GLASS_ENV_EMA").ok().as_deref() != Some("0"),
            env_tau: 0.15,
            frost_half_mode: {
                let on = std::env::var("GLASS_FROST_HALF").ok().as_deref() == Some("1");
                if on {
                    eprintln!("[info] GLASS_FROST_HALF=1：frost 保持 1/2 分辨率（视觉档，非逐位一致）");
                }
                on
            },
            env_freeze: std::cell::Cell::new(false),
            down_shader,
            up_shader,
            env_shader,
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

    /// 环境色探针纹理：开启 EMA 时是最新一张 ping-pong 结果，
    /// 否则退回 1/16 层本身（与旧实现逐位一致）。
    pub fn env_rt(&self) -> Option<&RenderTarget> {
        let i = self.env_latest.get();
        if self.env_enabled && i >= 0 {
            return self.env_rt.get(i as usize);
        }
        self.down.get(LEVELS - 1)
    }

    /// 环境色 EMA pass：对 `region`（全屏坐标）内的 1/16 层做时域平滑。
    /// `region` 用块足迹而不是 Pass A 的刷新区，因为足迹在块静止时是稳定的，
    /// EMA 历史才不会被反复重置；足迹外的 down[3] 是"更旧但有效"的数据，
    /// 混合进EMA 恰好就是它应有的收敛值。
    /// 运行期切换 frost 半分辨率档（自适应质量控制器 level 1 用）。
    /// 切换会重建金字塔 RT 并使全部缓存失效，是一次几十 ms 的停顿，
    /// 因此只允许在档位变化时调用，不允许每帧调用。
    pub fn frost_half_mode(&self) -> bool {
        self.frost_half_mode
    }

    /// 切换 level 1 的半分辨率 frost 档。
    ///
    /// **只重建 `frost`/`soft`** —— 这是唯一与档位相关的资源。
    /// 旧实现把 `width` 置 0 强制走整链 `resize()`，会连带销毁并重建
    /// `down`(4 张) / `frost_half`(2 张) / `env_rt`(2 张)，而它们与档位完全无关；
    /// 重建期间新旧两套 RT 在驱动侧同时驻留，实测双屏 Private 峰值 +42 MB
    /// （158.7 -> 200.2 MB），而 level 1 的本意是**省** 26 MB。
    pub fn set_frost_half_mode(
        &mut self,
        device: &ID3D11Device,
        ctx: &ID3D11DeviceContext,
        on: bool,
    ) -> Result<(), String> {
        if self.frost_half_mode == on {
            return Ok(());
        }
        self.frost_half_mode = on;
        if self.width == 0 {
            // 还没建过任何资源：下一次 resize 会直接按新模式建。
            return Ok(());
        }
        if on {
            let Some(half) = self.frost_half.first().cloned() else {
                self.frost_half_mode = false;
                return Err("frost 半分辨率链尚未创建".to_string());
            };
            // 先放下整屏 frost 再指向半分辨率链，避免两张同时驻留
            self.frost = None;
            self.soft = None;
            self.soft = Some(half.clone());
            self.frost = Some(half);
        } else {
            let (w, h) = (self.width.max(4), self.height.max(4));
            let frost = make_rt(device, w, h, DXGI_FORMAT_B8G8R8A8_UNORM)?;
            self.frost = None;
            self.soft = None;
            self.soft = Some(frost.clone());
            self.frost = Some(frost);
        }
        // D3D11 的绑定是强引用：上下文里还绑着的旧 RTV/SRV 不摘掉，
        // 旧纹理就不会真正释放（驱动侧继续驻留）。切档是低频事件，摘一次即可。
        unsafe {
            let null: [Option<ID3D11ShaderResourceView>; 9] =
                [None, None, None, None, None, None, None, None, None];
            ctx.PSSetShaderResources(0, Some(&null));
            ctx.OMSetRenderTargets(None, None);
        }
        Ok(())
    }

    pub fn set_env_freeze(&self, v: bool) {
        self.env_freeze.set(v);
    }

    fn run_env_ema(&self, ctx: &ID3D11DeviceContext, region: [i32; 4]) {
        if !self.env_enabled || self.env_rt.len() != 2 || self.env_freeze.get() {
            return;
        }
        let Some(src) = self.down.get(LEVELS - 1) else {
            return;
        };
        let latest = self.env_latest.get();
        let next = if latest < 0 { 0 } else { 1 - latest as usize };
        let first = latest < 0 || {
            let r = self.env_region.get();
            !(r[0] <= region[0] && r[1] <= region[1] && r[2] >= region[2] && r[3] >= region[3])
        };
        let now = std::time::Instant::now();
        let dt = now.saturating_duration_since(self.env_last.get()).as_secs_f32();
        self.env_last.set(now);
        let alpha = if first {
            1.0
        } else {
            1.0 - (-dt.max(1e-4) / self.env_tau.max(1e-3)).exp()
        };

        let dst = &self.env_rt[next];
        let scissor = self.scissor_for(region, dst);
        let cb_value = BlurCb {
            texel: [
                1.0 / src.w.max(1) as f32,
                1.0 / src.h.max(1) as f32,
                dst.w as f32 / src.w.max(1) as f32,
                dst.h as f32 / src.h.max(1) as f32,
            ],
            weight_center: alpha,
            _pad: [if first { 1.0 } else { 0.0 }, 0.0, 0.0],
        };
        if update_cbuffer(ctx, &self.cb, &cb_value).is_err() {
            return;
        }
        unsafe {
            let null: [Option<ID3D11ShaderResourceView>; 9] =
                [None, None, None, None, None, None, None, None, None];
            ctx.PSSetShaderResources(0, Some(&null));
            ctx.OMSetRenderTargets(None, None);
            ctx.OMSetRenderTargets(Some(&[Some(dst.rtv.clone())]), None);
            ctx.RSSetScissorRects(Some(&[scissor]));
            ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: dst.w as f32,
                Height: dst.h as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            ctx.VSSetShader(&self.env_shader.vs, None);
            ctx.PSSetShader(&self.env_shader.ps, None);
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
            let prev_srv = if latest < 0 {
                None
            } else {
                Some(self.env_rt[latest as usize].srv.clone())
            };
            ctx.PSSetShaderResources(0, Some(&[Some(src.srv.clone()), prev_srv]));
            ctx.Draw(3, 0);
        }
        self.env_latest.set(next as i8);
        self.env_region.set(if first {
            region
        } else {
            let r = self.env_region.get();
            [
                r[0].min(region[0]),
                r[1].min(region[1]),
                r[2].max(region[2]),
                r[3].max(region[3]),
            ]
        });
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
        if self.frost_half_mode {
            // 半分辨率档：省掉整屏上采样那一步，也省掉一张整屏 RT
            let half = self.frost_half[0].clone();
            self.soft = Some(half.clone());
            self.frost = Some(half);
        } else {
            let frost = make_rt(device, w, h, DXGI_FORMAT_B8G8R8A8_UNORM)?;
            // soft 与 frost 内容一致，共享同一份资源
            self.soft = Some(frost.clone());
            self.frost = Some(frost);
        }
        // 环境色探针：两张 1/16 RT 做 ping-pong EMA
        self.env_rt.clear();
        let (ew, eh) = (self.down[LEVELS - 1].w, self.down[LEVELS - 1].h);
        for _ in 0..2 {
            self.env_rt
                .push(make_rt(device, ew, eh, DXGI_FORMAT_B8G8R8A8_UNORM)?);
        }
        self.env_latest.set(-1);
        self.env_region.set([0, 0, 0, 0]);
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
        env_region: [i32; 4],
        env_advance: bool,
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
        // 局部重算必须「幂等」：否则同一区域反复重算会得到不同结果，
        // 表现为玻璃背景隔帧来回跳变（实测 ±1~3 LSB 整块震荡，偶尔 max 110）。
        // 下采样链：第 k 级的 4-tap 读第 k-1 级 ±1 texel = 全屏 ±2^k px，
        // 因此第 k 级的 scissor 外扩 2^(k+1) px，保证读到的都是本 run 或「未变化捕获」的内容。
        let grow = |r: [i32; 4], g: i32| [r[0] - g, r[1] - g, r[2] + g, r[3] + g];
        // --- 下采样 ---
        // 幂等所需的逐级外扩：第 k 级的 4-tap 读第 k-1 级 ±1 texel = ±2^k px，
        // 递归展开得 g_{k-1} ≥ g_k + 2^k。取 g3=0 → g2=8 → g1=12 → g0=14。
        // 旧的 2/4/8/16 在 level 0/1 上不足，会在 scissor 边界留下 ±1~3 LSB
        // 的残差带（verify_dirty 间歇 FAIL 的来源）。
        const DOWN_GROW: [i32; LEVELS] = [14, 12, 8, 0];
        {
            let sc = self.scissor_for(grow(region, DOWN_GROW[0]), &self.down[0]);
            self.draw(ctx, capture_srv, &self.down[0], w, h, sc);
        }
        for k in 1..LEVELS {
            let src = self.down[k - 1].srv.clone();
            let (sw, sh) = (self.down[k - 1].w, self.down[k - 1].h);
            let sc = self.scissor_for(grow(region, DOWN_GROW[k]), &self.down[k]);
            self.draw(ctx, &src, &self.down[k], sw, sh, sc);
        }

        // --- 环境色探针时域平滑（方案 12.6）---
        // 只有「捕获内容真的变了」才推进 EMA。拖拽/足迹跳变触发的 Pass A
        // 重算并不改变背景，若 EMA 仍推进，每次都会给全部块一步 ±1~2 LSB
        // 的色调跳变 —— 每帧重画时就是用户看到的「其他玻璃颤抖」。
        if env_advance {
            self.run_env_ema(ctx, env_region);
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
                // ping-pong 链的 scissor 必须逐 pass 收缩：pass p 只能读 pass p-1 本 run
                // 刷新过的范围。tent 半径按半分辨 8 px（全屏 16 px）估算，
                // 于是 pass 0 外扩 32、之后每 pass 减 8，最后一 pass 正好覆盖 region。
                for pass in 0..4 {
                    let src_i = pass & 1;
                    let dst_i = 1 - src_i;
                    let src = self.frost_half[src_i].srv.clone();
                    let (sw, sh) = (self.frost_half[src_i].w, self.frost_half[src_i].h);
                    let g = 32 - 8 * pass as i32;
                    let sc = self.scissor_for(grow(region, g), &self.frost_half[dst_i]);
                    self.draw_up(ctx, &src, &self.frost_half[dst_i], sw, sh, sc);
                    unsafe {
                        let null = [None::<ID3D11ShaderResourceView>];
                        ctx.PSSetShaderResources(0, Some(&null));
                        ctx.OMSetRenderTargets(None, None);
                    }
                }
                // 最终半分辨率结果上采样回整屏。soft 与 frost 共享，只需一次。
                // 半分辨率档直接跳过这一步：frost 就是 frost_half[0]。
                if !self.frost_half_mode {
                    let s1 = self.frost_half[0].srv.clone();
                    let (sw, sh) = (self.frost_half[0].w, self.frost_half[0].h);
                    let sc_full = self.scissor_for(region, frost);
                    self.draw_up(ctx, &s1, frost, sw, sh, sc_full);
                }
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
