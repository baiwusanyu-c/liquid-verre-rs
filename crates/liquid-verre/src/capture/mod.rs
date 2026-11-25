//! 采集层统一封装（方案 4.1 / 4.3 / 10.5 `pick_backend()`）。
//!
//! 两路后端：
//!   - **DDA**（DXGI Desktop Duplication）：提供 `GetFrameDirtyRects` 脏矩形，
//!     能把拷贝与 Pass A 都收窄到真正变化的区域；方案 12.3 也指出它通常比 WGC
//!     低 2~5 ms 延迟。**优先**。
//!   - **WGC**（Windows.Graphics.Capture）：支持多路并发采集，不受"每个 output
//!     全局只允许一个 duplication"限制；DDA 被录屏软件占用、被独占全屏打断、
//!     或返回 `E_ACCESSDENIED` 时**回退**到它。
//!
//! `GLASS_BACKEND=dda|wgc` 可强制指定其中一路（排障 / 对比用）。
//! `GLASS_SYNTH_CAP=1` 用一张**确定性合成图案**覆盖采集结果，使跨进程、跨时间的
//! 视觉对比可复现（真实桌面每次都不一样，无法做逐像素取证）。

pub mod dda;
pub mod wgc;

pub use dda::DdaCapture;
pub use wgc::WgcCapture;

use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE,
    D3D11_CPU_ACCESS_FLAG, D3D11_CPU_ACCESS_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
    D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8X8_UNORM,
    DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Gdi::HMONITOR;

pub enum Backend {
    Dda(DdaCapture),
    Wgc(WgcCapture),
}

/// 确定性合成采集源（仅取证用）
struct Synth {
    tex: ID3D11Texture2D,
    w: u32,
    h: u32,
}

pub struct Capture {
    backend: Backend,
    synth_enabled: bool,
    synth: Option<Synth>,
    /// 0.5.4 黑帧探针的复用回读位图（只在会话头几帧的探针里出现，结论后
    /// `drop_probe_buffers` 释放；4K staging 是 33 MB 级承诺，不白占）。
    probe_stage: Option<ID3D11Texture2D>,
}

pub type CaptureFrame = (ID3D11Texture2D, i32, i32, bool);

impl Capture {
    /// 按方案 10.5 的顺序挑后端；两路都失败时把两边的错误一起返回，便于定位。
    /// 按「环境变量 > 上层配置 > 自动规则」的优先级选后端并创建采集会话。
    ///
    /// 允许上层通过 [`crate::api::config::CaptureBackend`]
    /// 表达偏好。`GLASS_BACKEND` 环境变量仍然优先（它是排障后门，必须能压过一切配置）。
    pub fn for_monitor_pref(
        device: &ID3D11Device,
        hmonitor: HMONITOR,
        pref: crate::api::config::CaptureBackend,
    ) -> crate::error::Result<Self> {
        let forced = std::env::var("GLASS_BACKEND").ok().or_else(|| match pref {
            crate::api::config::CaptureBackend::ForceDda => Some("dda".to_string()),
            crate::api::config::CaptureBackend::ForceWgc => Some("wgc".to_string()),
            crate::api::config::CaptureBackend::Auto => None,
        });
        let want_wgc = forced.as_deref() == Some("wgc");
        let want_dda = forced.as_deref() == Some("dda");

        // HDR / 宽色域输出（色彩空间 ≠ SDR G22/P709）：DDA 的复制面给的是
        // HDR 参考值（实测整张桌面过曝，R/G/B 顶到 255 的比例 52%/70%/84%）。
        // WGC 请求 B8G8R8A8 时由 DWM 完成 HDR->SDR（实测与 SDR 桌面逐统计一致：
        // 平均亮度 178.0、裁切 1.8/1.2/0.7%），因此这种输出上**默认走 WGC**。
        let cs = dda::output_color_space(device, hmonitor);
        let hdr_out = cs.is_some_and(|c| c != 0);
        if hdr_out && !want_dda {
            log::info!("检测到 HDR/宽色域输出（色彩空间 = {}）：DDA 复制面是 HDR 参考值，采集改走 WGC，由 DWM 完成 HDR->SDR",
                cs.unwrap_or(0)
            );
        }
        let prefer_wgc = want_wgc || (hdr_out && !want_dda);

        let backend = if prefer_wgc {
            match WgcCapture::new_for_monitor(device, hmonitor) {
                Ok(c) => Backend::Wgc(c),
                Err(wgc_err) => {
                    if want_wgc {
                        return Err(format!("WGC: {wgc_err}").into());
                    }
                    match DdaCapture::new_for_monitor(device, hmonitor) {
                        Ok(c) => {
                            log::warn!(
                                "WGC 不可用（{wgc_err}），HDR 输出上回退到 DDA（背景可能过曝）"
                            );
                            Backend::Dda(c)
                        }
                        Err(e2) => return Err(format!("WGC: {wgc_err} | DDA: {e2}").into()),
                    }
                }
            }
        } else {
            match DdaCapture::new_for_monitor(device, hmonitor) {
                Ok(c) => Backend::Dda(c),
                Err(dda_err) => {
                    if want_dda {
                        return Err(
                            format!("DDA: {dda_err} | WGC: 已被 GLASS_BACKEND=dda 禁用").into()
                        );
                    }
                    match WgcCapture::new_for_monitor(device, hmonitor) {
                        Ok(c) => {
                            log::info!("DDA 不可用（{dda_err}），已回退到 WGC 采集");
                            Backend::Wgc(c)
                        }
                        Err(e2) => return Err(format!("DDA: {dda_err} | WGC: {e2}").into()),
                    }
                }
            }
        };
        Ok(Self {
            backend,
            synth_enabled: std::env::var("GLASS_SYNTH_CAP").ok().as_deref() == Some("1"),
            synth: None,
            probe_stage: None,
        })
    }

    /// 0.5.4 黑帧探针降级专用：**强制**用 DDA 重建会话（DDA 失败即 Err，不回退 WGC）。
    ///
    /// 不能复用 `for_monitor_pref(ForceDda)`：`GLASS_BACKEND` 后门的语义是
    /// 「必须能压过一切配置」（见该函数文档），在 `GLASS_BACKEND=wgc +
    /// GLASS_BLACK_PROBE=force` 的取证场景里它会把「降级」的新会话又建回 WGC，
    /// 降级空转。降级动作本身要绕过后门：要么 DDA，要么失败让上层提示用户。
    pub fn downgrade_to_dda(
        device: &ID3D11Device,
        hmonitor: HMONITOR,
    ) -> crate::error::Result<Self> {
        Ok(Self {
            backend: Backend::Dda(DdaCapture::new_for_monitor(device, hmonitor)?),
            synth_enabled: std::env::var("GLASS_SYNTH_CAP").ok().as_deref() == Some("1"),
            synth: None,
            probe_stage: None,
        })
    }

    pub fn backend_name(&self) -> &'static str {
        match self.backend {
            Backend::Dda(_) => "dda",
            Backend::Wgc(_) => "wgc",
        }
    }

    /// 确定性图案：只由像素坐标决定，与真实桌面无关，因此两次运行的采集逐位一致。
    fn ensure_synth(&mut self, device: &ID3D11Device, w: u32, h: u32) -> crate::error::Result<()> {
        if let Some(s) = &self.synth {
            if s.w == w && s.h == h {
                return Ok(());
            }
        }
        let mut pixels = vec![0u8; (w as usize) * (h as usize) * 4];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                // 低频渐变 + 中频格纹 + 高频伪随机，三种频率都要覆盖到，
                // 才能同时检验模糊、折射位移与色散。
                let fx = x as f32 / w as f32;
                let fy = y as f32 / h as f32;
                let grad_r = 40.0 + 180.0 * fx;
                let grad_g = 30.0 + 160.0 * fy;
                let grad_b = 90.0 + 120.0 * (1.0 - fx);
                let grid = if (x / 48 + y / 48) % 2 == 0 {
                    26.0
                } else {
                    -18.0
                };
                let band = 40.0 * ((fx * 9.0 + fy * 4.0) * std::f32::consts::TAU).sin();
                let noise = (((x.wrapping_mul(37) ^ y.wrapping_mul(91)) % 23) as f32) - 11.0;
                let clamp = |v: f32| (v.round().clamp(0.0, 255.0)) as u8;
                pixels[i] = clamp(grad_b + grid + band * 0.5 + noise);
                pixels[i + 1] = clamp(grad_g + grid - band * 0.3 + noise);
                pixels[i + 2] = clamp(grad_r + grid + band + noise);
                pixels[i + 3] = 255;
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
            CPUAccessFlags: D3D11_CPU_ACCESS_FLAG(0).0 as u32,
            MiscFlags: 0,
        };
        let sub = D3D11_SUBRESOURCE_DATA {
            pSysMem: pixels.as_ptr() as *const _,
            SysMemPitch: w * 4,
            SysMemSlicePitch: w * h * 4,
        };
        let mut tex = None;
        unsafe {
            device
                .CreateTexture2D(&desc, Some(&sub), Some(&mut tex))
                .map_err(|e| format!("CreateTexture2D(synth): {e}"))?;
        }
        let tex = tex.ok_or("synth texture null")?;
        log::info!("GLASS_SYNTH_CAP=1：使用确定性合成采集源 {w}x{h}");
        self.synth = Some(Synth { tex, w, h });
        Ok(())
    }

    /// 阻塞点：桌面有新内容立即返回，静止时在驱动/内核里等待（方案 12.4.1）。
    pub fn try_capture(
        &mut self,
        device: &ID3D11Device,
        ctx: &ID3D11DeviceContext,
        timeout_ms: u32,
    ) -> crate::error::Result<Option<CaptureFrame>> {
        let frame = match &mut self.backend {
            Backend::Dda(c) => c.try_capture(device, ctx, timeout_ms)?,
            Backend::Wgc(c) => c.try_capture(device, ctx, timeout_ms)?,
        };
        let Some((tex, w, h, changed)) = frame else {
            return Ok(None);
        };
        if self.synth_enabled {
            self.ensure_synth(device, w.max(1) as u32, h.max(1) as u32)?;
            if let Some(s) = &self.synth {
                unsafe { ctx.CopyResource(&tex, &s.tex) };
            }
        }
        Ok(Some((tex, w, h, changed)))
    }

    /// 本帧桌面脏区并集（output 局部像素）。`None` = 后端没提供（WGC）或
    /// 内容变了但拿不到脏矩形 —— 调用方必须按"整块足迹都变了"处理。
    pub fn dirty_union(&self) -> Option<[i32; 4]> {
        if self.synth_enabled {
            // 合成源恒定不变：等效于"没有任何脏区"
            return Some([0, 0, 0, 0]);
        }
        match &self.backend {
            Backend::Dda(c) => c.dirty_union,
            Backend::Wgc(c) => c.dirty_union,
        }
    }

    pub fn copied_px(&self) -> i64 {
        match &self.backend {
            Backend::Dda(c) => c.copied_px,
            Backend::Wgc(c) => c.copied_px,
        }
    }

    /// 后端是否报告「采集源已失效」。
    ///
    /// 目前只由 `DdaCapture` 提供，`try_capture` 的 `Err` 分支已经覆盖了同样的信息，
    /// 因此保留但不参与判定 —— 留着是给 `tools/verify_all.py` 的 ACCESS_LOST 取证用的。
    #[allow(dead_code)]
    pub fn access_lost(&self) -> bool {
        match &self.backend {
            Backend::Dda(c) => c.access_lost,
            Backend::Wgc(c) => c.access_lost,
        }
    }

    /// 采集段三段细分耗时（µs）：(AcquireNextFrame, 脏矩形读取, 像素搬运, acquire 峰值)
    /// 采集段细分（µs）：acquire / 脏矩形 API / 像素搬运 / 取帧前置 /
    /// ReleaseFrame / acquire 峰值 / ReleaseFrame 峰值
    pub fn take_timings(&mut self) -> [u128; 7] {
        match &mut self.backend {
            Backend::Dda(c) => {
                let r = [
                    c.t_acq_us,
                    c.t_dirty_us,
                    c.t_copy_us,
                    c.t_pre_us,
                    c.t_rel_us,
                    c.t_acq_max_us,
                    c.t_rel_max_us,
                ];
                c.t_acq_us = 0;
                c.t_dirty_us = 0;
                c.t_copy_us = 0;
                c.t_pre_us = 0;
                c.t_rel_us = 0;
                c.t_acq_max_us = 0;
                c.t_rel_max_us = 0;
                r
            }
            // WGC 后端没有脏矩形与显式拷贝分段
            Backend::Wgc(_) => [0; 7],
        }
    }

    /// 取出并清零统计：(acquire 次数, 真实内容变化次数)
    pub fn take_stats(&mut self) -> (u64, u64) {
        match &mut self.backend {
            Backend::Dda(c) => {
                let r = (c.acquired, c.content_changed);
                c.acquired = 0;
                c.content_changed = 0;
                c.pointer_only = 0;
                r
            }
            Backend::Wgc(c) => {
                let r = (c.acquired, c.content_changed);
                c.acquired = 0;
                c.content_changed = 0;
                c.pointer_only = 0;
                r
            }
        }
    }

    // ----- 0.5.4 黑帧探针（会话级，一次判定，绝不进稳态热路径）-----

    /// 当前会话是否值得武装黑帧探针（`runtime` 状态机在武装前询问）。
    ///
    /// - `GLASS_SYNTH_CAP=1`：合成图案不是真实采集，黑帧结论没有意义，
    ///   还会打断逐像素取证（合成图案恒定，探针必须让路）；
    /// - DDA 开着 `GLASS_SKIP_COPY / GLASS_SKIP_DIRTY` 归因后门：owned 纹理
    ///   是陈帧或不搬像素，「全黑」会变成假阳性。
    #[must_use]
    pub fn probe_armable(&self) -> bool {
        if self.synth_enabled {
            return false;
        }
        match &self.backend {
            Backend::Dda(c) => c.probe_copy_honest(),
            Backend::Wgc(_) => true,
        }
    }

    /// 探测最近一次交付帧：`(是否全黑, 网格摘要)`。`None` = 还没有帧 / 格式不支持探测。
    ///
    /// ⚠ `Map` 是**阻塞**回读（等 GPU 拷贝完成），4K 屏毫秒级停顿：只允许
    /// 黑帧探针状态机在会话头几帧调用（每会话 ≤4 次），稳态路径永不触碰。
    /// 探针读的是后端**自有纹理**（我们自己 CopyResource 出来的副本），
    /// 与 DDA 借用纹理的 ReleaseFrame 时序无关，任何时点采样都自洽。
    pub fn last_frame_black_probe(
        &mut self,
        device: &ID3D11Device,
        ctx: &ID3D11DeviceContext,
    ) -> Option<(bool, u64)> {
        let owned = match &self.backend {
            Backend::Dda(c) => c.last_owned()?,
            Backend::Wgc(c) => c.last_owned()?,
        };
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: `desc` 是栈上结构，按 API 约定由引用写出。
        unsafe { owned.GetDesc(&mut desc) };
        let (bpp, color_bytes) = probe_layout(desc.Format)?;
        let (w, h) = (desc.Width as usize, desc.Height as usize);
        if w < 4 || h < 4 {
            return None;
        }
        let stage = self.ensure_probe_stage(device, &desc)?;
        // SAFETY: 两张纹理同尺寸同格式（ensure_probe_stage 保证）且属于本 device。
        unsafe { ctx.CopyResource(&stage, &owned) };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: stage 是刚确保过的 STAGING/READ 纹理，子资源 0 存在。
        let ok = unsafe { ctx.Map(&stage, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }.is_ok();
        if !ok {
            return None;
        }
        let pitch = mapped.RowPitch as usize;
        let len = pitch.saturating_mul(h);
        // SAFETY: STAGING 映射区大小 = RowPitch × Height（D3D11 契约），
        // `pData` 只在 Map/Unmap 之间有效，本切片同样只在这个窗口里用。
        let data = unsafe { std::slice::from_raw_parts(mapped.pData as *const u8, len) };
        let r = grid_probe(
            data,
            pitch,
            w,
            h,
            ProbeCfg {
                bpp,
                color_bytes,
                x_step: PROBE_X_STEP,
                y_step: PROBE_Y_STEP,
                threshold: PROBE_THRESHOLD,
            },
        );
        // SAFETY: 与上面的 Map 成对（同纹理同子资源）。
        unsafe { ctx.Unmap(&stage, 0) };
        Some(r)
    }

    /// 探针结论落定后释放回读位图（重建廉价：真要用再 `ensure_probe_stage`）。
    pub fn drop_probe_buffers(&mut self) {
        self.probe_stage = None;
    }

    /// 创建/复用与后端自有纹理同尺寸同格式的 STAGING 回读位图。
    fn ensure_probe_stage(
        &mut self,
        device: &ID3D11Device,
        src: &D3D11_TEXTURE2D_DESC,
    ) -> Option<ID3D11Texture2D> {
        if let Some(t) = &self.probe_stage {
            let mut d = D3D11_TEXTURE2D_DESC::default();
            // SAFETY: `d` 栈上结构，引用写出。
            unsafe { t.GetDesc(&mut d) };
            if d.Width == src.Width && d.Height == src.Height && d.Format == src.Format {
                return Some(t.clone());
            }
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
            ..*src
        };
        let mut tex = None;
        // SAFETY: desc 由后端真实纹理派生，STAGING+READ+BindFlags=0 是合法组合。
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut tex)) }.ok()?;
        let tex = tex?;
        self.probe_stage = Some(tex.clone());
        Some(tex)
    }
}

// ---------------------------------------------------------------------------
// 0.5.4 采集黑帧探针（Win10 的两条实测事故路径：
//   A. WGC 会把「被 `WDA_EXCLUDEFROMCAPTURE` 排除的全屏窗口」整体涂黑 ——
//      本库 `dda.rs` 模块头自证；HDR 输出自动切 WGC、DDA 创建失败回退 WGC 都撞得上；
//   B. MPO 硬件叠加层不进复制帧 —— Wallpaper Engine 视频壁纸，壁纸区域为黑。
// 判据本体是这里的纯函数（可单测），动作状态机在 `runtime`（会话头几帧，一次判定）。
// ---------------------------------------------------------------------------

/// 探针网格步长（像素）：两轴每 4 取 1。要辨的是**整幅涂黑**（结构性全黑），
/// 不是零星亮点 —— 网格够把「有正常内容」与「整幅黑」分开，回读扫描又不至于
/// 全帧级开销（4K ≈ 10 万采样点，亚毫秒）。
const PROBE_X_STEP: usize = 4;
const PROBE_Y_STEP: usize = 4;
/// 「非黑」通道字节阈值。⚠ 不是 0：DWM 合成后桌面带 dither 噪声，阈值太小会把
/// 「黑噪声」当内容；太大又吞掉真实的深色壁纸。4 = SDR dither 的容忍档。
const PROBE_THRESHOLD: u8 = 4;

/// 网格探针对一种像素格式的全部口径（[`probe_layout`] 的派生 + 采样常量）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ProbeCfg {
    bpp: usize,
    color_bytes: usize,
    x_step: usize,
    y_step: usize,
    threshold: u8,
}

/// 黑帧探针支持格式的 `(每像素字节数, 需检查的颜色字节数)`；`None` = 不碰探针。
///
/// 颜色字节从通道 0 起算：4B 的 BGRA/RGBA 看 3 字节（alpha 天然任意，不看）；
/// 8B 的 FP16 RGBA 看 6 字节（黑色 = 0x0000，无需解码指数域）。
/// 认不出的格式**宁可不探**：拿 4B 的步进去读 8B 像素是错步采样，结论比缺失更糟。
fn probe_layout(format: DXGI_FORMAT) -> Option<(usize, usize)> {
    if format == DXGI_FORMAT_B8G8R8A8_UNORM
        || format == DXGI_FORMAT_R8G8B8A8_UNORM
        || format == DXGI_FORMAT_R8G8B8A8_UNORM_SRGB
        || format == DXGI_FORMAT_B8G8R8X8_UNORM
    {
        Some((4, 3))
    } else if format == DXGI_FORMAT_R16G16B16A16_FLOAT {
        Some((8, 6))
    } else {
        None
    }
}

/// 黑帧判据本体：对映射区做网格采样，返回 `(是否全黑, 采样字节的 FNV-1a 摘要)`。
///
/// - `data` 是映射的原始字节（`pitch × h`），`bpp`/`color_bytes` 来自 [`probe_layout`]；
/// - 全黑 = 所有采样颜色字节 ≤ `threshold`；没有采样点（退化 w/h）⇒ `true` ——
///   「一个内容像素都没观测到」正是黑帧的定义（外层已保证 w,h ≥ 4，这里只是纯函数自洽）；
/// - 摘要用于「摘除画布采集排除前后帧内容是否改变」的裁决：两次调用必须同尺寸
///   同 cfg，否则视为改变（保守方向 = 判「排除涂黑」，宁可降级也不把黑底当正常背景）；
/// - 全部索引走 `get()`：映射是外部边界，pitch/尺寸的组合不可信，判据不许 panic。
fn grid_probe(data: &[u8], pitch: usize, w: usize, h: usize, cfg: ProbeCfg) -> (bool, u64) {
    let ProbeCfg {
        bpp,
        color_bytes,
        x_step,
        y_step,
        threshold,
    } = cfg;
    // FNV-1a 64 位：offset basis 起步，逐字节混合。
    let mut digest: u64 = 0xcbf2_9ce4_8422_2325;
    let mut all_black = true;
    if bpp == 0 || color_bytes == 0 || x_step == 0 || y_step == 0 {
        return (true, digest);
    }
    let mut gy = 0usize;
    while gy < h {
        let mut gx = 0usize;
        while gx < w {
            let off = gy
                .saturating_mul(pitch)
                .saturating_add(gx.saturating_mul(bpp));
            if let Some(px) = data.get(off..off.saturating_add(color_bytes)) {
                for &b in px {
                    digest ^= u64::from(b);
                    digest = digest.wrapping_mul(0x0000_0100_0000_01b3);
                    if b > threshold {
                        all_black = false;
                    }
                }
            }
            gx += x_step;
        }
        gy += y_step;
    }
    (all_black, digest)
}

#[cfg(test)]
mod tests {
    use super::{grid_probe, probe_layout, ProbeCfg};
    use windows::Win32::Graphics::Dxgi::Common::{
        DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8X8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT,
        DXGI_FORMAT_R32_FLOAT, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
    };

    #[test]
    fn probe_layout_covers_known_formats_only() {
        assert_eq!(probe_layout(DXGI_FORMAT_B8G8R8A8_UNORM), Some((4, 3)));
        assert_eq!(probe_layout(DXGI_FORMAT_R8G8B8A8_UNORM), Some((4, 3)));
        assert_eq!(probe_layout(DXGI_FORMAT_R8G8B8A8_UNORM_SRGB), Some((4, 3)));
        assert_eq!(probe_layout(DXGI_FORMAT_B8G8R8X8_UNORM), Some((4, 3)));
        assert_eq!(probe_layout(DXGI_FORMAT_R16G16B16A16_FLOAT), Some((8, 6)));
        assert_eq!(
            probe_layout(DXGI_FORMAT_R32_FLOAT),
            None,
            "未知格式宁可不探：错步采样比缺失更糟"
        );
    }

    /// 测试口径：4B/px、看 3 色字节、网格 4×4、阈值 4（与生产常量同形）；
    /// 个别用例用 1×1 步长覆盖 alpha/逐像素分支。
    fn cfg4() -> ProbeCfg {
        ProbeCfg {
            bpp: 4,
            color_bytes: 3,
            x_step: 4,
            y_step: 4,
            threshold: 4,
        }
    }

    fn cfg1() -> ProbeCfg {
        ProbeCfg {
            x_step: 1,
            y_step: 1,
            ..cfg4()
        }
    }

    #[test]
    fn all_zero_frame_is_black_and_digest_is_stable() {
        // 16x8、4B/px、pitch=64。同内容两次摘要必须逐位相同 —— 这是「摘除排除前后
        // 对比」的锚点，摘要不稳整个裁决就全是假阳性。
        let data = vec![0u8; 64 * 8];
        let (black0, d0) = grid_probe(&data, 64, 16, 8, cfg4());
        let (black1, d1) = grid_probe(&data, 64, 16, 8, cfg4());
        assert!(black0 && black1, "全零帧必须是黑帧");
        assert_eq!(d0, d1);
    }

    #[test]
    fn single_bright_grid_point_breaks_black_and_changes_digest() {
        let mut data = vec![0u8; 64 * 8];
        // 落在格点上：gy=4（步长 4）、gx=8（步长 4），只点亮 B 通道一个字节。
        data[4 * 64 + 8 * 4] = 200;
        let (black, d) = grid_probe(&data, 64, 16, 8, cfg4());
        assert!(!black, "一个格点有内容就足以否定黑帧");
        let (_, d0) = grid_probe(&[0u8; 64 * 8], 64, 16, 8, cfg4());
        assert_ne!(d, d0, "内容变了摘要必须变（裁决判据）");
    }

    #[test]
    fn off_grid_bright_pixel_does_not_matter_by_design() {
        // 语义钉死（别偷偷改成全扫描然后纳闷探针怎么老超时）：
        // 要辨的是整幅涂黑 vs 正常桌面，零星落在网格外的亮点**不是**辨别对象。
        let mut data = vec![0u8; 64 * 8];
        data[4] = 255; // gx=1（字节偏移 4）不在 4 步长网格上
        let (black, _) = grid_probe(&data, 64, 16, 8, cfg4());
        assert!(black);
    }

    #[test]
    fn alpha_bytes_are_invisible_to_the_probe() {
        // 画布/桌面的 alpha 天然任意：两帧 RGB 全同、只有 alpha 不同 ⇒
        // 同摘要、同「黑」结论。若把 alpha 卷进摘要，翻转裁决会被 alpha 噪声打穿。
        let mut a = vec![0u8; 64 * 8];
        let mut b = vec![0u8; 64 * 8];
        for row in a.chunks_mut(64) {
            for px in row.chunks_exact_mut(4) {
                px[3] = 12;
            }
        }
        for row in b.chunks_mut(64) {
            for px in row.chunks_exact_mut(4) {
                px[3] = 250;
            }
        }
        let (ba, da) = grid_probe(&a, 64, 16, 8, cfg1());
        let (bb, db) = grid_probe(&b, 64, 16, 8, cfg1());
        assert!(ba && bb);
        assert_eq!(da, db);
    }

    #[test]
    fn threshold_tolerates_dither_noise() {
        let data = vec![3u8; 64 * 8];
        assert!(
            grid_probe(&data, 64, 16, 8, cfg4()).0,
            "≤4 视为 dither 噪声"
        );
        let mut hot = vec![0u8; 64 * 8];
        hot[0] = 5;
        assert!(
            !grid_probe(&hot, 64, 16, 8, cfg4()).0,
            "越过阈值就不再是黑帧"
        );
    }

    #[test]
    fn degenerate_inputs_never_panic() {
        // 映射是外部边界：空数据 / 0 步长 / pitch 过小越界，全部退化为安全结论。
        let empty: [u8; 0] = [];
        assert!(grid_probe(&empty, 0, 0, 0, cfg4()).0);
        assert!(grid_probe(&empty, 64, 16, 8, ProbeCfg { bpp: 0, ..cfg4() }).0);
        assert!(
            grid_probe(
                &empty,
                64,
                16,
                8,
                ProbeCfg {
                    x_step: 0,
                    ..cfg4()
                }
            )
            .0
        );
        // pitch 远小于 w*bpp：格点越界 ⇒ get() 跳过，不 panic。
        let data = vec![0u8; 8];
        assert!(grid_probe(&data, 4, 64, 64, cfg4()).0);
    }
}
