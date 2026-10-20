//! C 模式合成器：全屏画布 + 单交换链 + DComp（方案 12.1）。
//! 一次 Present、一次 Commit；块数再多也不增加交换链 / Present 次数。

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

use crate::gfx::device::Device;
use crate::gfx::glass_renderer::GlassRenderer;
use crate::gfx::gpu_timer::GpuTimer;
use crate::gfx::pyramid::Pyramid;
use crate::gfx::swapchain::CanvasSwapchain;
use crate::gfx::DComp;
use crate::glass::block::Block;
use crate::glass::params::Params;

/// 交换链后备缓冲：只有 RTV（flip-model 后备缓冲不允许创建 SRV）
struct BackbufferRt {
    tex: ID3D11Texture2D,
    rtv: ID3D11RenderTargetView,
    w: i32,
    h: i32,
}

/// 矩形基础运算：脏区并集、金字塔有效区账本、影响盒判定都建在它们上。
/// 抽成自由函数是为了能单测（重构的质量阀门之一）。
pub fn rect_contains(outer: &[i32; 4], inner: &[i32; 4]) -> bool {
    outer[0] <= inner[0] && outer[1] <= inner[1] && outer[2] >= inner[2] && outer[3] >= inner[3]
}
pub fn rect_union(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}
pub fn rects_bbox(rs: &[[i32; 4]]) -> Option<[i32; 4]> {
    rs.iter()
        .fold(None, |acc, r| Some(acc.map_or(*r, |o| rect_union(o, *r))))
}
pub fn rects_hit(a: &[i32; 4], b: &[i32; 4]) -> bool {
    a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
}

pub struct Compositor {
    swap: CanvasSwapchain,
    rt: BackbufferRt,
    dcomp: DComp,
    pub pyramid: Pyramid,
    capture_tex: Option<ID3D11Texture2D>,
    capture_srv: Option<ID3D11ShaderResourceView>,
    capture_generation: u64,
    fallback_srv: Option<ID3D11ShaderResourceView>,
    pub width: i32,
    pub height: i32,
    /// 共享背景金字塔是否仍对应当前捕获帧。
    /// WGC 新帧到达时由 set_capture 置 false；鼠标移动只执行 Pass B。
    background_valid: bool,
    /// CPU submission/wait durations only, NOT GPU execution times.
    pub last_stage_us: [u128; 4], // Pass A, Pass B, Present, Commit
    /// 真实 GPU 计时（方案 7.6）。仅 GLASS_METRICS=1 时创建。
    pub gpu: Option<GpuTimer>,
    /// 上一帧各块的绘制矩形（画布像素），用于推导本帧脏区。
    prev_rects: Vec<(u32, [i32; 4])>,
    /// 本帧矩形 scratch（复用，热路径零分配）。
    cur_rects: Vec<(u32, [i32; 4])>,
    /// 本帧脏区统计（像素），用于 metrics。
    pub last_dirty_px: i64,
    /// 本帧是否真的画了块。
    pub last_drew_blocks: usize,
    /// 本帧是否重跑了 Pass A（背景真的变了）。
    pub last_pass_a: bool,
    /// 金字塔当前「内容有效」的全屏坐标矩形。块足迹超出它才需要重跑 Pass A。
    pyramid_valid: Option<[i32; 4]>,
    /// 本帧 Pass A 实际覆盖的像素数（metrics）。
    pub last_pass_a_px: i64,
    /// Pass A 限频（方案 12.4.3 的一档）。`None` = 不限频，此时行为与
    /// 未引入本机制时**逐位一致**：每个桌面变化帧都重算金字塔。
    ///
    /// 限频只影响 `frost`（重模糊层）与 1/16 环境色探针的新鲜度；清晰层
    /// `gCapture`、块几何、折射位移、色散、高光、鼠标点光、Pass B 全部仍然
    /// 每帧更新，位置/形状/光照保持零延迟。
    pass_a_min_interval: Option<std::time::Duration>,
    last_pass_a_at: std::time::Instant,
    /// 因限频被跳过的 Pass A 次数（metrics）
    pub pass_a_throttled: u64,
    /// 调试截图需要整屏内容，强制全量重画。
    force_full_shot: bool,
    /// GLASS_FORCE_FULL=1：关闭脏矩形优化，每帧整屏重画（A/B 对照用）。
    force_full_always: bool,
    /// 只作用于下一帧的全量重画（自证用，渲染后自动清除）。
    force_full_once: bool,
    /// GLASS_INTERNAL_SHOT_AT_MS：延迟到该时刻之后的第一帧再截图，
    /// 用于验证脏矩形路径（首帧本来就是整屏重画，测不出差异）。
    shot_at: Option<std::time::Instant>,
    /// 已经提交过至少一帧（首帧必须全量）。
    has_presented: bool,
    /// 性能归因开关：分别跳过 Pass A / Pass B / Present，用来定位 GPU 瓶颈。
    /// 会让画面不正确，**只用于实验**，默认全关。
    pub skip_pass_a: bool,
    pub skip_pass_b: bool,
    pub skip_present: bool,
    pub skip_clear: bool,
    /// GLASS_INTERNAL_SHOT 的输出路径（拍完即取走）。
    pub shot_path: Option<String>,
    /// 是否已经拍出这一帧（供 GLASS_VERIFY_PASS_A 自检串联第二次全量渲染）。
    pub shot_fired: bool,
    /// `GLASS_SHOT_EVERY_MS=<n>`：连拍模式，每 n ms 拍一张（序号进文件名）。
    /// 连拍时**不**强制全量重画（保留增量路径，那才是用户看到的路径），
    /// 且只写 composed 一个文件，用于逐帧分析「其他块为什么会变」。
    shot_every: Option<std::time::Duration>,
    shot_seq: u32,
    /// `GLASS_VERIFY_VALIDITY=1`：自检两类「漏更新」，只打告警、不改渲染。
    /// 用户看到的「背景闪动/颤抖」只可能来自这两类：
    ///   1) 金字塔有效性虚报：把没重算过的区域也标记为有效；
    ///   2) 该重画的块没进脏区：光标扫过 / 位置变化的块被跳过一帧。
    verify_validity: bool,
    pub vv_overclaim: u64,
    pub vv_skipped: u64,
    vv_last_log: std::time::Instant,
}

impl Compositor {
    pub fn new(gfx: &Device, hwnd: HWND, width: i32, height: i32) -> Result<Self, String> {
        let swap = CanvasSwapchain::new(gfx, hwnd, width, height)?;
        let rt = Self::make_backbuffer_rt(gfx, &swap, width, height)?;
        let dcomp = DComp::new(gfx, hwnd)?;
        let _visual = dcomp.attach_swapchain(&swap.swap)?;
        let pyramid = Pyramid::new(&gfx.device)?;

        // fallback 纹理：仅在采集完全不可用时兜底（main 在首帧就绪前不会显示窗口，
        // 因此正常路径永远不会采样它）。做成 8x8 渐变即可，省掉旧实现里
        // 1024x1024 的 4 MB 常驻显存与启动时约 100 万次 sin/cos 的 CPU 开销。
        let fallback_srv = Self::create_fallback_srv(gfx)?;
        // 注意：D3D11 timestamp query 会打断驱动的命令批处理，本身就会拖慢
        // 管线，因此独立于 GLASS_METRICS，只在专门做 GPU 归因时打开。
        let gpu = if std::env::var("GLASS_GPU_TIMINGS").ok().as_deref() == Some("1") {
            match GpuTimer::new(&gfx.device) {
                Ok(t) => Some(t),
                Err(e) => {
                    eprintln!("[warn] GPU timestamp query 不可用: {e}");
                    None
                }
            }
        } else {
            None
        };

        Ok(Self {
            swap,
            rt,
            dcomp,
            pyramid,
            capture_tex: None,
            capture_srv: None,
            capture_generation: 0,
            fallback_srv: Some(fallback_srv),
            width,
            height,
            background_valid: false,
            last_stage_us: [0; 4],
            gpu,
            prev_rects: Vec::new(),
            cur_rects: Vec::new(),
            last_dirty_px: 0,
            last_drew_blocks: 0,
            last_pass_a: false,
            verify_validity: std::env::var("GLASS_VERIFY_VALIDITY").ok().as_deref() == Some("1"),
            vv_overclaim: 0,
            vv_skipped: 0,
            vv_last_log: std::time::Instant::now(),
            pyramid_valid: None,
            last_pass_a_px: 0,
            pass_a_min_interval: {
                let hz: f64 = std::env::var("GLASS_PASS_A_HZ")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0.0);
                if hz > 0.0 {
                    eprintln!("[info] Pass A 限频 = {hz:.0} Hz（GLASS_PASS_A_HZ）");
                    Some(std::time::Duration::from_secs_f64(1.0 / hz))
                } else {
                    None
                }
            },
            last_pass_a_at: std::time::Instant::now().checked_sub(std::time::Duration::from_secs(3600)).unwrap_or_else(std::time::Instant::now),
            pass_a_throttled: 0,
            force_full_shot: std::env::var("GLASS_INTERNAL_SHOT").is_ok()
                && std::env::var("GLASS_SHOT_EVERY_MS").is_err(),
            force_full_always: std::env::var("GLASS_FORCE_FULL").ok().as_deref() == Some("1"),
            force_full_once: false,
            // 未指定延迟时默认「首帧即拍」，与旧实现一致。
            shot_at: std::env::var("GLASS_INTERNAL_SHOT").ok().map(|_| {
                let delay = std::env::var("GLASS_INTERNAL_SHOT_AT_MS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0);
                std::time::Instant::now() + std::time::Duration::from_millis(delay)
            }),
            has_presented: false,
            skip_pass_a: std::env::var("GLASS_SKIP_PASS_A").ok().as_deref() == Some("1"),
            skip_pass_b: std::env::var("GLASS_SKIP_PASS_B").ok().as_deref() == Some("1"),
            skip_present: std::env::var("GLASS_SKIP_PRESENT").ok().as_deref() == Some("1"),
            skip_clear: std::env::var("GLASS_SKIP_CLEAR").ok().as_deref() == Some("1"),
            shot_path: std::env::var("GLASS_INTERNAL_SHOT").ok(),
            shot_fired: false,
            shot_every: std::env::var("GLASS_SHOT_EVERY_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .map(std::time::Duration::from_millis),
            shot_seq: 0,
        })
    }

    /// 创建 fallback SRV：极小的柔和渐变，仅在采集彻底不可用时兜底。
    fn create_fallback_srv(gfx: &Device) -> Result<ID3D11ShaderResourceView, String> {
        let w = 8u32;
        let h = 8u32;
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let cx = x as f32 / (w - 1).max(1) as f32;
                let cy = y as f32 / (h - 1).max(1) as f32;
                pixels[i] = (100.0 + 120.0 * cx * 0.5 + 40.0 * cy) as u8;
                pixels[i + 1] = (90.0 + 120.0 * cy * 0.5 + 50.0 * cx) as u8;
                pixels[i + 2] = (130.0 + 80.0 * (1.0 - cx) + 40.0 * cy) as u8;
                pixels[i + 3] = 255u8;
            }
        }

        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };

        let sub = D3D11_SUBRESOURCE_DATA {
            pSysMem: pixels.as_ptr() as *const _,
            SysMemPitch: w * 4,
            SysMemSlicePitch: w * h * 4,
        };

        let mut tex = None;
        unsafe {
            gfx.device
                .CreateTexture2D(&desc, Some(&sub), Some(&mut tex))
                .map_err(|e| format!("CreateTexture2D(fallback): {e}"))?;
        }
        let tex = tex.ok_or("fallback texture")?;

        let mut srv = None;
        unsafe {
            gfx.device
                .CreateShaderResourceView(&tex, None, Some(&mut srv))
                .map_err(|e| format!("CreateShaderResourceView(fallback): {e}"))?;
        }
        srv.ok_or("fallback srv".to_string())
    }

    fn make_backbuffer_rt(
        gfx: &Device,
        swap: &CanvasSwapchain,
        w: i32,
        h: i32,
    ) -> Result<BackbufferRt, String> {
        let tex: ID3D11Texture2D = swap.backbuffer(&gfx.device)?;
        let mut rtv = None;
        unsafe {
            gfx.device
                .CreateRenderTargetView(&tex, None, Some(&mut rtv))
                .map_err(|e| format!("CreateRenderTargetView: {e}"))?;
        }
        Ok(BackbufferRt {
            tex,
            rtv: rtv.ok_or("rtv")?,
            w,
            h,
        })
    }

    pub fn resize(&mut self, gfx: &Device, w: i32, h: i32) -> Result<(), String> {
        if self.width == w
            && self.height == h
            && self.pyramid.width() == w
            && self.pyramid.height() == h
        {
            return Ok(());
        }
        if self.width == w && self.height == h {
            self.pyramid.resize(&gfx.device, w, h)?;
            return Ok(());
        }
        self.swap.resize(gfx, w, h)?;
        self.rt = Self::make_backbuffer_rt(gfx, &self.swap, w, h)?;
        self.width = w;
        self.height = h;
        self.pyramid.resize(&gfx.device, w, h)?;
        self.background_valid = false;
        self.pyramid_valid = None;
        // ResizeBuffers 之后后备缓冲内容未定义，必须整屏重画一次。
        self.prev_rects.clear();
        self.has_presented = false;
        Ok(())
    }

    /// 自适应质量 level 1：frost 半分辨率档的运行期开关。
    pub fn set_frost_half(&mut self, gfx: &Device, on: bool) -> Result<bool, String> {
        let before = self.pyramid.frost_half_mode();
        self.pyramid.set_frost_half_mode(&gfx.device, &gfx.ctx, on)?;
        if before != on {
            self.invalidate();
        }
        Ok(before != on)
    }

    /// 采集源重建后调用：金字塔与后备缓冲都失效，下一帧必须整屏重来。
    pub fn invalidate(&mut self) {
        self.pyramid_valid = None;
        self.background_valid = false;
        self.prev_rects.clear();
        self.has_presented = false;
    }

    /// 解除 Pass A 限频的一次性豁免：自证对照帧必须跑完整金字塔。
    pub fn pass_a_now(&mut self) {
        self.last_pass_a_at =
            std::time::Instant::now().checked_sub(std::time::Duration::from_secs(3600)).unwrap_or_else(std::time::Instant::now);
    }

    /// 自检用：把下一帧强制变成「全量 Pass A + 全量 Pass B」。
    pub fn force_full_next(&mut self) {
        self.pyramid_valid = None;
        self.background_valid = false;
        self.prev_rects.clear();
        self.has_presented = false;
        self.force_full_once = true;
        self.pass_a_now();
        // 同一帧第二次渲染必须复用完全相同的环境色 EMA 结果
        self.pyramid.set_env_freeze(true);
        self.shot_at = Some(std::time::Instant::now());
    }

    pub fn capture_ready(&self) -> bool {
        self.capture_srv.is_some()
    }

    pub fn set_capture(
        &mut self,
        gfx: &Device,
        tex: ID3D11Texture2D,
        refresh_background: bool,
    ) -> Result<(), String> {
        // DDA 返回的是 compositor 自己的长期纹理；只创建一次 SRV，避免每个
        // desktop frame 在 CPU 热路径重复创建 COM view。
        let reuse = self.capture_tex.as_ref().is_some_and(|old| old == &tex);
        if !reuse {
            let mut srv = None;
            unsafe {
                gfx.device
                    .CreateShaderResourceView(&tex, None, Some(&mut srv))
                    .map_err(|e| format!("CreateShaderResourceView(capture): {e}"))?;
            }
            self.capture_srv = srv;
            self.capture_tex = Some(tex);
        }
        self.capture_generation = self.capture_generation.wrapping_add(1);
        // 仅在确实检测到桌面内容变化时刷新共享背景，拖拽/鼠标帧不触发 Pass A。
        if refresh_background {
            self.background_valid = false;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    /// 渲染一个显示器画布。`blocks` 必须已经是**该画布局部坐标**，
    /// `order` 是 `blocks` 的下标（z 升序）—— 多屏时每个画布各自一份。
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        gfx: &Device,
        glass: &GlassRenderer,
        blocks: &[Block],
        order: &[usize],
        params: &Params,
        cursor_screen: [f32; 2],
        canvas_origin: [f32; 2],
        drag_velocity: [f32; 2],
        time: f32,
        press: f32,
        hover_id: Option<u32>,
        prev_cursor_canvas: [f32; 2],
        full_redraw: bool,
        backdrop_dirty: Option<[i32; 4]>,
    ) -> Result<(), String> {
        // 优先使用 WGC 采集帧，无帧时用 fallback 纹理保证玻璃可见。
        // capture_srv 的所有权仍由 compositor 持有，Pass A 只在它失效时更新。
        let capture_srv = self
            .capture_srv
            .clone()
            .or_else(|| self.fallback_srv.clone());
        let Some(capture_srv) = capture_srv else {
            return self.present();
        };
        if let Some(gpu) = &mut self.gpu {
            gpu.begin_frame(&gfx.ctx);
            gpu.mark(&gfx.ctx, 0);
        }

        // ---- 脏区推导（方案 12.4.1 / 12.5 的矩形级实现）----
        // flip-model（FLIP_SEQUENTIAL）后备缓冲逐 buffer 保留内容，因此只需把
        // 「本帧真正会变的像素」清成透明黑并重画，其余像素沿用上一帧。
        let cursor_canvas = [
            cursor_screen[0] - canvas_origin[0],
            cursor_screen[1] - canvas_origin[1],
        ];
        let margin = params.shadow_margin.max(0.0);
        self.cur_rects.clear();
        for &index in order {
            let Some(block) = blocks.get(index) else {
                continue;
            };
            let rt_w = (block.width + margin * 2.0).ceil().max(1.0);
            let rt_h = (block.height + margin * 2.0).ceil().max(1.0);
            let sx = (block.x - margin).floor() as i32;
            let sy = (block.y - margin).floor() as i32;
            self.cur_rects.push((
                block.id,
                [sx, sy, sx + rt_w as i32, sy + rt_h as i32],
            ));
        }

        let shot_now = self
            .shot_at
            .map_or(false, |at| std::time::Instant::now() >= at);
        if shot_now {
            self.shot_at = None;
        }
        // Pass A 只需要覆盖「块真正会采样到的像素」：
        //   块矩形 ⊕ (折射位移 + 色散 + 模糊核 + 速度外推) 的余量。
        let vel = (drag_velocity[0] * drag_velocity[0]
            + drag_velocity[1] * drag_velocity[1])
            .sqrt();
        let pad = (128.0 + (vel * params.lag_seconds).min(128.0)) as i32;
        let mut need: Option<[i32; 4]> = None;
        for &(_, r) in self.cur_rects.iter() {
            let e = [r[0] - pad, r[1] - pad, r[2] + pad, r[3] + pad];
            need = Some(match need {
                None => e,
                Some(o) => [
                    o[0].min(e[0]),
                    o[1].min(e[1]),
                    o[2].max(e[2]),
                    o[3].max(e[3]),
                ],
            });
        }
        let need = need.map(|r| {
            [
                r[0].clamp(0, self.width),
                r[1].clamp(0, self.height),
                r[2].clamp(0, self.width),
                r[3].clamp(0, self.height),
            ]
        });

        let backdrop_changed = !self.background_valid;
        // Pass A 的刷新区域（方案 7.4-2 脏矩形金字塔 + 12.5 足迹相交）：
        //   桌面变了  -> 只重算「DDA 脏矩形 ⊕ 模糊核」∩「块足迹 ⊕ 采样余量」；
        //                拿不到脏矩形时保守退回整块足迹。
        //   桌面没变  -> 只有块移动导致足迹外扩时才重算，并外扩 192 px 做迟滞，
        //                拖拽一小段距离不必重跑金字塔。
        const BLUR_REACH: i32 = 96;
        const HYSTERESIS: i32 = 192;
        let clamp_rect = |r: [i32; 4]| -> [i32; 4] {
            [
                r[0].clamp(0, self.width),
                r[1].clamp(0, self.height),
                r[2].clamp(0, self.width),
                r[3].clamp(0, self.height),
            ]
        };
        let covered = match (need, self.pyramid_valid) {
            (Some(n), Some(v)) => v[0] <= n[0] && v[1] <= n[1] && v[2] >= n[2] && v[3] >= n[3],
            (None, _) => true,
            (Some(_), None) => false,
        };
        let (run_rect, new_valid): (Option<[i32; 4]>, Option<[i32; 4]>) = match need {
            None => (None, self.pyramid_valid),
            // 足迹没有被现有金字塔完整覆盖（块移动/新建/切档重建，或 pad 因拖拽
            // 速度变大而外扩）：**必须整块重算**。这个分支必须排在 backdrop_changed 前面：
            // 旧实现让 backdrop_changed 抢先，只刷「桌面脏区 ∩ 足迹」就把**整个足迹**
            // 标记为已算有效 —— 从没算过的区域会一直沿用 RT 的旧内容甚至初值，
            // 玻璃里就出现「背景失真 / 局部过曝 / 随桌面更新一块块闪动」。
            // 实测（GLASS_VERIFY_VALIDITY=1 + SIM_BACKDROP + 跳变拖拽）：修复前 14 s 内虚报 39 帧。
            Some(n) if !covered => {
                let r = clamp_rect([
                    n[0] - HYSTERESIS,
                    n[1] - HYSTERESIS,
                    n[2] + HYSTERESIS,
                    n[3] + HYSTERESIS,
                ]);
                // 只声称本帧真正重算过的 r，**不与旧有效区合并**：旧有效区里可能有
                // 「桌面已变、但当时不在任何块足迹内因此没刷」的部分，合并进来就是虚报，
                // 等块移过去时会露出陈旧背景。r 自带 192 px 迟滞，连续拖拽时下一帧
                // 仍然 covered，不会退化成每帧全量重算。
                (Some(r), Some(r))
            }
            Some(n) if backdrop_changed => {
                // 刷新范围 = 「桌面脏区 ⊕ 模糊核」∩ **我们声称有效的区域**，而不只是当前足迹。
                // 只刷 ∩n 就必须把有效区收缩到 n，于是下一帧足迹一动又 !covered ->
                // 每帧整块重算；而高频的分块重算会让重模糊链在 scissor 边界读到
                // 陈旧 texel，玻璃里的背景出现「扫过式」抖动（verify_stale 实测
                // 17.9 万像素差异、最大通道差 54）。刷 ∩base 之后 base 内部全部与当前
                // capture 一致（脏区外本来就没变），有效区可以原样保留，不再来回收缩。
                // 进到这个分支时 covered 已为真，因此 base ⊇ n。
                let base = self.pyramid_valid.unwrap_or(n);
                let region = match backdrop_dirty {
                    Some(d) => {
                        let x = [
                            (d[0] - BLUR_REACH).max(base[0]),
                            (d[1] - BLUR_REACH).max(base[1]),
                            (d[2] + BLUR_REACH).min(base[2]),
                            (d[3] + BLUR_REACH).min(base[3]),
                        ];
                        if x[2] > x[0] && x[3] > x[1] {
                            Some(x)
                        } else {
                            None
                        }
                    }
                    None => Some(base),
                };
                (region, Some(base))
            }
            // covered 且桌面没变：金字塔仍与当前 capture 一致，不用重算。
            Some(_) => (None, self.pyramid_valid),
        };

        if self.cur_rects.is_empty() && self.prev_rects.is_empty() && !full_redraw {
            // 本屏一块玻璃都没有：整帧短路，不清屏、不 Pass B、不 Present、不 Commit。
            // 多屏时玻璃常常只集中在其中一两块屏，其余屏必须真正零成本。
            self.last_dirty_px = 0;
            self.last_drew_blocks = 0;
            self.last_pass_a = false;
            self.last_pass_a_px = 0;
            return Ok(());
        }

        // 桌面变化时不必整屏重画：只有「足迹与桌面脏区相交」的块才需要重画
        // （方案 12.5 的矩形级实现）。拿不到脏矩形时才退回整屏。
        let full = [0, 0, self.width, self.height];
        let force_full = self.force_full_always || self.force_full_once;
        self.force_full_once = false;
        let mut dirty: Option<[i32; 4]> = if full_redraw
            || force_full
            || (shot_now && self.force_full_shot)
            || !self.has_presented
            || self.prev_rects.len() != self.cur_rects.len()
        {
            Some(full)
        } else {
            None
        };
        let union = |d: &mut Option<[i32; 4]>, r: [i32; 4]| {
            *d = Some(match *d {
                None => r,
                Some(o) => rect_union(o, r),
            });
        };
        if dirty.is_none() && backdrop_changed {
            match backdrop_dirty {
                None => dirty = Some(full),
                Some(d) => {
                    // 块采样到的最远像素 = 块矩形 ⊕ pad（折射位移 + 色散 +
                    // 模糊核 + 速度外推）；桌面变化的影响范围再外扩一个模糊核。
                    let e = [
                        d[0] - BLUR_REACH - pad,
                        d[1] - BLUR_REACH - pad,
                        d[2] + BLUR_REACH + pad,
                        d[3] + BLUR_REACH + pad,
                    ];
                    for &(_, r) in self.cur_rects.iter() {
                        if r[0] < e[2] && r[2] > e[0] && r[1] < e[3] && r[3] > e[1] {
                            union(&mut dirty, r);
                        }
                    }
                }
            }
        }
        // 位移与光标影响盒必须**始终**参与并集。旧代码把它们包在
        // `if dirty.is_none()` 里，于是只要这一帧桌面有变化（本机实测 27~116 次/秒），
        // 光标扫过的块与刚移动的块就被整帧跳过 —— 内容停更一帧甚至多帧再补，
        // 用户看到的就是「鼠标在玻璃里移动时背景/边缘颤抖」、「拖一块、其他块闪动」。
        // 实测（GLASS_VERIFY_VALIDITY=1）：修复前 14 s 内漏画 774 次。
        {
            // 位置/尺寸变化的块：旧矩形 ∪ 新矩形都要刷新
            for &(id, r) in self.cur_rects.iter() {
                match self.prev_rects.iter().find(|(pid, _)| *pid == id) {
                    None => union(&mut dirty, r),
                    Some(&(_, pr)) if pr != r => {
                        union(&mut dirty, pr);
                        union(&mut dirty, r);
                    }
                    Some(_) => {}
                }
            }
            // 鼠标点光/悬停高光是屏幕空间场：光标扫过的块必须重画
            // （旧位置要擦掉余晖，新位置要画上余晖）。
            // 必须与 glass_ps.hlsl 里 gate 的支撑半径（3*mouse_radius）一致：
            // 盒外块的鼠标依赖精确为 0，盒内块整块重画，两者都不会出现拼接缝。
            let influence = (params.mouse_radius * 3.0 + 32.0) as i32;
            let box_x0 = (prev_cursor_canvas[0].min(cursor_canvas[0]).floor() as i32) - influence;
            let box_y0 = (prev_cursor_canvas[1].min(cursor_canvas[1]).floor() as i32) - influence;
            let box_x1 = (prev_cursor_canvas[0].max(cursor_canvas[0]).ceil() as i32) + influence;
            let box_y1 = (prev_cursor_canvas[1].max(cursor_canvas[1]).ceil() as i32) + influence;
            for &(_, r) in self.cur_rects.iter() {
                if r[0] < box_x1 && r[2] > box_x0 && r[1] < box_y1 && r[3] > box_y0 {
                    union(&mut dirty, r);
                }
            }
        }
        if self.verify_validity {
            let contains = |d: Option<[i32; 4]>, r: [i32; 4]| -> bool {
                d.map_or(false, |o| rect_contains(&o, &r))
            };
            // (2) 光标影响盒内 / 位置变化的块，必须被脏区完整包住
            let infl = crate::glass::params::cursor_influence_px(params.mouse_radius);
            let bx0 = (prev_cursor_canvas[0].min(cursor_canvas[0]).floor() as i32) - infl;
            let by0 = (prev_cursor_canvas[1].min(cursor_canvas[1]).floor() as i32) - infl;
            let bx1 = (prev_cursor_canvas[0].max(cursor_canvas[0]).ceil() as i32) + infl;
            let by1 = (prev_cursor_canvas[1].max(cursor_canvas[1]).ceil() as i32) + infl;
            for &(id, r) in self.cur_rects.iter() {
                let in_box = r[0] < bx1 && r[2] > bx0 && r[1] < by1 && r[3] > by0;
                let moved = match self.prev_rects.iter().find(|(pid, _)| *pid == id) {
                    None => true,
                    Some(&(_, pr)) => pr != r,
                };
                if (in_box || moved) && !contains(dirty, r) {
                    self.vv_skipped += 1;
                }
            }
            // (1) 声称有效的区域必须落在「之前已有效 ∪ 本帧重算」里
            if let Some(v) = new_valid {
                let mut allv: Vec<[i32; 4]> = Vec::new();
                if let Some(a) = self.pyramid_valid {
                    allv.push(a);
                }
                if let Some(b) = run_rect {
                    allv.push(b);
                }
                let u = rects_bbox(&allv);
                if !contains(u, v) {
                    self.vv_overclaim += 1;
                }
            }
            if self.vv_overclaim + self.vv_skipped > 0
                && self.vv_last_log.elapsed() >= std::time::Duration::from_secs(1)
            {
                self.vv_last_log = std::time::Instant::now();
                eprintln!(
                    "[verify] 漏更新自检：金字塔虚报有效 {} 帧 / 该重画却漏掉的块 {} 次",
                    self.vv_overclaim, self.vv_skipped
                );
            }
        }
        std::mem::swap(&mut self.prev_rects, &mut self.cur_rects);
        self.cur_rects.clear();

        let Some(dirty_rect) = dirty else {
            // 没有任何像素变化：不 Present、不 Commit（事件驱动的终点）。
            self.last_dirty_px = 0;
            self.last_drew_blocks = 0;
            self.last_pass_a = false;
            self.last_pass_a_px = 0;
            self.pyramid_valid = new_valid;
            self.background_valid = true;
            return Ok(());
        };
        let dirty_rect = [
            dirty_rect[0].clamp(0, self.width),
            dirty_rect[1].clamp(0, self.height),
            dirty_rect[2].clamp(0, self.width),
            dirty_rect[3].clamp(0, self.height),
        ];
        if dirty_rect[2] <= dirty_rect[0] || dirty_rect[3] <= dirty_rect[1] {
            self.last_dirty_px = 0;
            self.last_drew_blocks = 0;
            self.last_pass_a = false;
            self.last_pass_a_px = 0;
            self.pyramid_valid = new_valid;
            self.background_valid = true;
            return Ok(());
        }
        self.last_dirty_px = (dirty_rect[2] - dirty_rect[0]) as i64
            * (dirty_rect[3] - dirty_rect[1]) as i64;

        // 只把脏区清成透明黑；其余像素沿用上一帧的同一张后备缓冲。
        unsafe {
            gfx.ctx
                .OMSetRenderTargets(Some(&[Some(self.rt.rtv.clone())]), None);
            gfx.ctx.RSSetState(glass.raster_state());
            gfx.ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: self.rt.w as f32,
                Height: self.rt.h as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            gfx.ctx.RSSetScissorRects(Some(&[windows::Win32::Foundation::RECT {
                left: dirty_rect[0],
                top: dirty_rect[1],
                right: dirty_rect[2],
                bottom: dirty_rect[3],
            }]));
            if !self.skip_clear {
                // 每帧自包含：flip 模型的 back buffer 在 Present 后内容不保，
                // 因此「只清脏区 + 只画脏块」不成立（没画的块会整块消失 = 用户看到的闪动）。
                // 改为整屏清 + 画全部块；Pass A 仍按脏区 scissor（那才是成本大头）。
                gfx.ctx
                    .ClearRenderTargetView(&self.rt.rtv, &[0.0, 0.0, 0.0, 0.0]);
            }
        }
                // Pass A：全屏共享模糊金字塔，但只刷新 run_rect 内的像素。
        // 鼠标/拖拽只进入 Pass B，不触发整屏 down/up-sample。
        // Pass A 限频：被跳过的帧沿用上一份金字塔，且**不**把 background_valid
        // 置真，这样下一帧会继续尝试，直到限频窗口过去。
        let throttled = run_rect.is_some()
            && !self.skip_pass_a
            && self
                .pass_a_min_interval
                .map_or(false, |iv| self.last_pass_a_at.elapsed() < iv);
        if throttled {
            self.pass_a_throttled += 1;
            self.last_stage_us[0] = 0;
            self.last_pass_a_px = 0;
            self.last_pass_a = false;
        } else {
            if let Some(region) = run_rect.filter(|_| !self.skip_pass_a) {
                let t = std::time::Instant::now();
                // 环境色 EMA 用「块足迹」而不是本次刷新区：足迹在块静止时稳定，
                // EMA 历史才不会被反复重置（方案 12.6）。
                self.pyramid.run(
                    &gfx.ctx,
                    &capture_srv,
                    self.width,
                    self.height,
                    region,
                    need.unwrap_or(region),
                    backdrop_changed,
                );
                self.last_stage_us[0] = t.elapsed().as_micros();
                self.last_pass_a_px =
                    (region[2] - region[0]) as i64 * (region[3] - region[1]) as i64;
                self.last_pass_a = true;
                self.last_pass_a_at = std::time::Instant::now();
            } else {
                self.last_stage_us[0] = 0;
                self.last_pass_a_px = 0;
                self.last_pass_a = false;
            }
            self.pyramid_valid = new_valid;
            self.background_valid = true;
        }
        self.pyramid.set_env_freeze(false);
        if let Some(gpu) = &mut self.gpu {
            gpu.mark(&gfx.ctx, 1);
        }
        // Pass B：逐块 scissor 绘制（只画与脏区相交的块）
        let pass_b_start = std::time::Instant::now();
        if !self.skip_pass_b {
        glass.draw_blocks(
            &gfx.ctx,
            &self.rt.rtv,
            self.rt.w,
            self.rt.h,
            &self.pyramid,
            &capture_srv,
            blocks,
            order,
            None,
            params,
            self.width as f32,
            self.height as f32,
            canvas_origin,
            cursor_screen,
            drag_velocity,
            time,
            press,
            hover_id,
        );
        }
        self.last_stage_us[1] = pass_b_start.elapsed().as_micros();
        self.last_drew_blocks = glass.last_drawn();
        self.has_presented = true;
        if let Some(gpu) = &mut self.gpu {
            gpu.mark(&gfx.ctx, 2);
        }
        if shot_now {
            let burst = self.shot_every.is_some();
            let path_opt = if burst {
                self.shot_path.clone().map(|p| {
                    let seq = self.shot_seq;
                    self.shot_seq += 1;
                    let stem = p.strip_suffix(".bmp").unwrap_or(p.as_str());
                    format!("{stem}.{seq:03}.bmp")
                })
            } else {
                self.shot_path.take()
            };
            if let Some(path) = path_opt {
                if let Err(e) = self.save_debug_bmp(gfx, &path, burst) {
                    eprintln!("[warn] internal glass screenshot failed: {e}");
                } else {
                    self.shot_fired = true;
                    if burst {
                        if let Some(every) = self.shot_every {
                            self.shot_at = Some(std::time::Instant::now() + every);
                        }
                    }
                    eprintln!("[info] saved internal glass frame: {path}");
                }
            }
        }
        let result = self.present();
        if let Some(gpu) = &mut self.gpu {
            gpu.mark(&gfx.ctx, 3);
            gpu.end_frame(&gfx.ctx);
        }
        result
    }

    /// 保存三类诊断图：
    /// - path：最终合成预览（capture 背景 + premultiplied overlay）
    /// - path.overlay.bmp：原始 overlay backbuffer
    /// - path.capture.bmp：原始桌面捕获纹理
    /// 只有第一张与 DComp 最终视觉语义一致。
    pub fn save_debug_bmp(
        &self,
        gfx: &Device,
        path: &str,
        composed_only: bool,
    ) -> Result<(), String> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { self.rt.tex.GetDesc(&mut desc); }
        let staging_desc = D3D11_TEXTURE2D_DESC {
            Width: desc.Width,
            Height: desc.Height,
            MipLevels: 1,
            ArraySize: 1,
            Format: desc.Format,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let staging = unsafe {
            let mut out = None;
            gfx.device
                .CreateTexture2D(&staging_desc, None, Some(&mut out))
                .map_err(|e| format!("CreateTexture2D(staging): {e}"))?;
            out.ok_or("CreateTexture2D(staging) returned null")?
        };
        unsafe {
            gfx.ctx.CopyResource(&staging, &self.rt.tex);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            gfx.ctx
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|e| format!("Map(staging): {e}"))?;
            let row_bytes = desc.Width as usize * 4;
            let mut pixels = Vec::with_capacity(row_bytes * desc.Height as usize);
            for y in 0..desc.Height as usize {
                let row = std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                    row_bytes,
                );
                // 诊断文件保留真实 premultiplied backbuffer，不在这里人为加白底。
                // 另一个工具链可将它按 DComp 语义合成到任意背景；否则会把
                // 取证图误当成最终桌面视觉，掩盖真实 alpha/黑边问题。
                pixels.extend_from_slice(row);
            }
            gfx.ctx.Unmap(&staging, 0);
            Self::write_bmp(path, desc.Width, desc.Height, &pixels)?;

            // Read the captured desktop and compose with the premultiplied overlay:
            // out.rgb = overlay.rgb + capture.rgb * (1 - overlay.a).
            if let Some(capture) = &self.capture_tex {
                let mut cd = D3D11_TEXTURE2D_DESC::default();
                capture.GetDesc(&mut cd);
                if cd.Width == desc.Width && cd.Height == desc.Height {
                    let capture_staging_desc = D3D11_TEXTURE2D_DESC {
                        Width: cd.Width, Height: cd.Height, MipLevels: 1, ArraySize: 1,
                        Format: cd.Format, SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                        Usage: D3D11_USAGE_STAGING, BindFlags: 0,
                        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32, MiscFlags: 0,
                    };
                    let mut capture_staging = None;
                    gfx.device.CreateTexture2D(
                        &capture_staging_desc, None, Some(&mut capture_staging),
                    ).map_err(|e| format!("Create capture staging: {e}"))?;
                    let capture_staging = capture_staging.ok_or("capture staging null")?;
                    gfx.ctx.CopyResource(&capture_staging, capture);
                    let mut cm = D3D11_MAPPED_SUBRESOURCE::default();
                    gfx.ctx.Map(&capture_staging, 0, D3D11_MAP_READ, 0, Some(&mut cm))
                        .map_err(|e| format!("Map capture staging: {e}"))?;
                    let mut capture_pixels = Vec::with_capacity(row_bytes * desc.Height as usize);
                    for y in 0..desc.Height as usize {
                        let row = std::slice::from_raw_parts(
                            (cm.pData as *const u8).add(y * cm.RowPitch as usize), row_bytes,
                        );
                        capture_pixels.extend_from_slice(row);
                    }
                    gfx.ctx.Unmap(&capture_staging, 0);

                    let mut composed = pixels.clone();
                    for (out, bg) in composed.chunks_exact_mut(4)
                        .zip(capture_pixels.chunks_exact(4))
                    {
                        let inv = 255u16.saturating_sub(out[3] as u16);
                        out[0] = (out[0] as u16 + bg[0] as u16 * inv / 255).min(255) as u8;
                        out[1] = (out[1] as u16 + bg[1] as u16 * inv / 255).min(255) as u8;
                        out[2] = (out[2] as u16 + bg[2] as u16 * inv / 255).min(255) as u8;
                        out[3] = 255;
                    }
                    let want_extra = !composed_only
                        || std::env::var("GLASS_BURST_OVERLAY").ok().as_deref() == Some("1");
                    if want_extra {
                        let overlay_path = format!("{path}.overlay.bmp");
                        let capture_path = format!("{path}.capture.bmp");
                        Self::write_bmp(&overlay_path, desc.Width, desc.Height, &pixels)?;
                        Self::write_bmp(&capture_path, desc.Width, desc.Height, &capture_pixels)?;
                    }
                    Self::write_bmp(path, desc.Width, desc.Height, &composed)?;
                    if want_extra {
                        if let Some(soft) = &self.pyramid.soft {
                            Self::save_texture_bmp(gfx, &soft.tex, &format!("{path}.soft.bmp"))?;
                        }
                        if let Some(frost) = &self.pyramid.frost {
                            Self::save_texture_bmp(gfx, &frost.tex, &format!("{path}.frost.bmp"))?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn save_texture_bmp(
        gfx: &Device,
        texture: &ID3D11Texture2D,
        path: &str,
    ) -> Result<(), String> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { texture.GetDesc(&mut desc); }
        let staging_desc = D3D11_TEXTURE2D_DESC {
            Width: desc.Width, Height: desc.Height, MipLevels: 1, ArraySize: 1,
            Format: desc.Format, SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING, BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32, MiscFlags: 0,
        };
        let mut staging = None;
        unsafe { gfx.device.CreateTexture2D(&staging_desc, None, Some(&mut staging)) }
            .map_err(|e| format!("Create staging {path}: {e}"))?;
        let staging = staging.ok_or("staging null")?;
        unsafe {
            gfx.ctx.CopyResource(&staging, texture);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            gfx.ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|e| format!("Map {path}: {e}"))?;
            let row_bytes = desc.Width as usize * 4;
            let mut pixels = Vec::with_capacity(row_bytes * desc.Height as usize);
            for y in 0..desc.Height as usize {
                let row = std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(y * mapped.RowPitch as usize), row_bytes,
                );
                pixels.extend_from_slice(row);
            }
            gfx.ctx.Unmap(&staging, 0);
            Self::write_bmp(path, desc.Width, desc.Height, &pixels)
        }
    }

    fn write_bmp(path: &str, width: u32, height: u32, pixels: &[u8]) -> Result<(), String> {
        let file_size = 54u32 + pixels.len() as u32;
        let mut bmp = Vec::with_capacity(file_size as usize);
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&file_size.to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&(54u32).to_le_bytes());
        bmp.extend_from_slice(&(40u32).to_le_bytes());
        bmp.extend_from_slice(&(width as i32).to_le_bytes());
        bmp.extend_from_slice(&(-(height as i32)).to_le_bytes());
        bmp.extend_from_slice(&(1u16).to_le_bytes());
        bmp.extend_from_slice(&(32u16).to_le_bytes());
        bmp.extend_from_slice(&[0; 24]);
        bmp.extend_from_slice(pixels);
        std::fs::write(path, bmp).map_err(|e| format!("write debug BMP {path}: {e}"))
    }

    fn present(&mut self) -> Result<(), String> {
        if self.skip_present {
            self.last_stage_us[2] = 0;
            self.last_stage_us[3] = 0;
            return Ok(());
        }
        let present_start = std::time::Instant::now();
        self.swap.present()?;
        self.last_stage_us[2] = present_start.elapsed().as_micros();
        let commit_start = std::time::Instant::now();
        let result = self.dcomp.commit();
        self.last_stage_us[3] = commit_start.elapsed().as_micros();
        result
    }
}

#[allow(dead_code)]
fn _unused(_t: ID3D11Texture2D) {
    let _ = DXGI_FORMAT_B8G8R8A8_UNORM;
}

#[cfg(test)]
mod tests {
    use super::{rect_contains, rect_union, rects_bbox, rects_hit};

    #[test]
    fn contains_is_inclusive() {
        assert!(rect_contains(&[0, 0, 100, 100], &[0, 0, 100, 100]));
        assert!(rect_contains(&[0, 0, 100, 100], &[10, 10, 90, 90]));
        assert!(!rect_contains(&[0, 0, 100, 100], &[90, 90, 110, 110]));
    }
    #[test]
    fn union_is_commutative_and_associative() {
        let a = [0, 0, 10, 10];
        let b = [5, 5, 20, 8];
        assert_eq!(rect_union(a, b), rect_union(b, a));
        assert_eq!(rect_union(a, b), [0, 0, 20, 10]);
        let c = [30, 0, 40, 40];
        assert_eq!(rect_union(rect_union(a, b), c), rect_union(a, rect_union(b, c)));
    }
    #[test]
    fn bbox_of_empty_is_none() {
        assert_eq!(rects_bbox(&[]), None);
        assert_eq!(rects_bbox(&[[1, 2, 3, 4]]), Some([1, 2, 3, 4]));
        assert_eq!(rects_bbox(&[[0, 0, 5, 5], [10, 10, 15, 12]]), Some([0, 0, 15, 12]));
    }
    #[test]
    fn hit_is_open_interval() {
        // 相切但不重叠不算相交（与脏区语义一致）
        assert!(rects_hit(&[0, 0, 10, 10], &[5, 5, 20, 20]));
        assert!(!rects_hit(&[0, 0, 10, 10], &[10, 0, 20, 20]));
        assert!(!rects_hit(&[0, 0, 10, 10], &[0, 10, 20, 20]));
    }
}
