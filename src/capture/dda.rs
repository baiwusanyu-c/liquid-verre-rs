//! DXGI Desktop Duplication capture.
//!
//! Unlike monitor WGC + WDA_EXCLUDEFROMCAPTURE, duplication does not replace an
//! excluded full-screen overlay with a black rectangle. The render thread copies
//! the acquired frame into an owned SRV texture before ReleaseFrame.

use windows::core::Interface;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11ShaderResourceView, ID3D11Texture2D,
    D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_CPU_ACCESS_FLAG, D3D11_CPU_ACCESS_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R16G16B16A16_FLOAT;
use windows::Win32::Graphics::Dxgi::{
    IDXGIDevice, IDXGIOutput1, IDXGIOutput6, IDXGIOutputDuplication, IDXGIResource,
    DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTDUPL_MOVE_RECT,
};

/// 超过这个数量就直接整屏拷贝：几十个 CopySubresourceRegion 的调用开销
/// 会超过一次全屏 DMA。
const MAX_DIRTY_RECTS: usize = 24;

/// 局部拷贝策略（`GLASS_COPY_MODE`），用于判定"N 次小拷贝"与"1 次并集大拷贝"
/// 哪个在本机驱动上更便宜：
///   multi（默认）= 逐脏矩形 CopySubresourceRegion，超过 24 个退回整屏
///   union        = 只拷脏矩形并集 bbox 一次
///   full         = 永远整屏 CopyResource
#[derive(Clone, Copy, PartialEq)]
pub enum CopyMode {
    Multi,
    Union,
    Full,
}

/// `GLASS_SIM_DIRTY=<w>x<h>@<n>`：用 n 个固定尺寸、确定性散布的脏矩形取代
/// 真实脏矩形，使"逐矩形拷 vs 并集拷 vs 整屏拷"的对比在完全相同的搬运量下进行。
fn sim_dirty(out_w: i32, out_h: i32) -> Option<Vec<[i32; 4]>> {
    let v = std::env::var("GLASS_SIM_DIRTY").ok()?;
    let (wh, n) = v.split_once('@')?;
    let (w, h) = wh.split_once('x')?;
    let (w, h, n) = (w.parse::<i32>().ok()?, h.parse::<i32>().ok()?, n.parse::<usize>().ok()?);
    if w <= 0 || h <= 0 || n == 0 {
        return None;
    }
    // 紧凑网格而不是全屏散布：散布会让并集 bbox 超过半屏，从而触发
    // "并集太大就退回整屏拷贝"的保护，实验就测不到逐矩形拷贝了。
    let cols = (n as f64).sqrt().ceil().max(1.0) as i32;
    let rows = ((n as i32) + cols - 1) / cols;
    let grid_w = (cols * w).min(out_w);
    let grid_h = (rows * h).min(out_h);
    let x0 = ((out_w - grid_w) / 2).max(0);
    let y0 = ((out_h - grid_h) / 2).max(0);
    let mut out = Vec::with_capacity(n);
    for i in 0..n as i32 {
        let x = x0 + (i % cols) * w;
        let y = y0 + (i / cols) * h;
        out.push([
            x.clamp(0, out_w),
            y.clamp(0, out_h),
            (x + w).clamp(0, out_w),
            (y + h).clamp(0, out_h),
        ]);
    }
    Some(out)
}

fn copy_mode() -> CopyMode {
    match std::env::var("GLASS_COPY_MODE").ok().as_deref() {
        Some("union") => CopyMode::Union,
        Some("full") => CopyMode::Full,
        _ => CopyMode::Multi,
    }
}

/// AcquireNextFrame 超时是**正常**的「桌面没变化」，不是错误。
const DXGI_ERROR_WAIT_TIMEOUT: i32 = 0x887A_0027u32 as i32;
/// 独占全屏切换 / 另一个进程接管 duplication / 显示模式变化都会让它失效，
/// 方案 4.3 要求检测并退避重试，否则主循环会退化成空转。
const DXGI_ERROR_ACCESS_LOST: i32 = 0x887A_0026u32 as i32;

fn union_add(u: &mut Option<[i32; 4]>, r: [i32; 4]) {
    *u = Some(match *u {
        None => r,
        Some(o) => [
            o[0].min(r[0]),
            o[1].min(r[1]),
            o[2].max(r[2]),
            o[3].max(r[3]),
        ],
    });
}

pub struct DdaCapture {
    duplication: IDXGIOutputDuplication,
    texture: Option<ID3D11Texture2D>,
    srv: Option<ID3D11ShaderResourceView>,
    pub width: i32,
    pub height: i32,
    last_frame_signature: u64,
    /// 诊断计数：AcquireNextFrame 成功次数 / 真实内容变化次数 / 纯指针更新次数。
    pub acquired: u64,
    pub content_changed: u64,
    pub pointer_only: u64,
    /// GetFrameDirtyRects / GetFrameMoveRects 的复用缓冲（热路径零分配）。
    meta: Vec<u8>,
    /// 本帧桌面脏矩形列表（output 局部像素坐标），用于局部 CopySubresourceRegion。
    pub dirty: Vec<[i32; 4]>,
    /// 本帧实际拷贝了多少像素（metrics）。
    pub copied_px: i64,
    /// duplication 已失效，需要由调用方重建（方案 4.3）。
    pub access_lost: bool,
    /// 本帧 dirty/copy 段耗时，用于从 t_pre 里扣掉（避免重复计入）
    t_dirty_us_last: u128,
    t_copy_us_last: u128,
    copy_mode: CopyMode,
    /// 归因用：跳过实际像素搬运，只保留 AcquireNextFrame + 脏矩形读取
    skip_copy: bool,
    /// 归因用：连脏矩形读取（GetFrameDirtyRects/MoveRects）也跳过，
    /// 用来把 cap 段的耗时精确归到 AcquireNextFrame 还是脏矩形 API 上。
    skip_dirty: bool,
    /// 采集段三段细分耗时累加（µs）：AcquireNextFrame / 脏矩形读取 / 像素搬运。
    /// 用来区分"驱动 API 阻塞"与"我们自己的拷贝开销"。
    pub t_acq_us: u128,
    pub t_dirty_us: u128,
    pub t_copy_us: u128,
    pub t_acq_max_us: u128,
    /// 取帧前置（QI/GetDesc/必要时重建纹理）与 ReleaseFrame
    pub t_pre_us: u128,
    pub t_rel_us: u128,
    pub t_rel_max_us: u128,
    /// GLASS_VERIFY_COPY=1：维护一张「永远整屏 CopyResource」的参考纹理，
    /// 周期性回读并与局部拷贝结果逐像素比对，自证脏矩形没有漏掉任何区域。
    verify: bool,
    verify_tex: Option<ID3D11Texture2D>,
    verify_stage_a: Option<ID3D11Texture2D>,
    verify_stage_b: Option<ID3D11Texture2D>,
    verify_frames: u64,
    /// 本帧桌面脏区并集（output 局部像素坐标）。
    /// `None` 表示「内容变了但拿不到脏矩形」，调用方必须按整屏处理。
    pub dirty_union: Option<[i32; 4]>,
    hdr_warned: bool,
}

/// 该显示器输出的色彩空间（DXGI_COLOR_SPACE_TYPE；0 = SDR G22/P709）。
/// HDR / 宽色域输出上 DDA 的复制面给的是 **HDR 参考值**（实测 HDR 开启时
/// 无论 FP16 还是 8bit 容器，R/G/B 顶到 255 的比例都是 52%/70%/84%，
/// 整张桌面死白），我们自己 tonemap 不如让 DWM 做：见 mod.rs 的自动切 WGC。
pub fn output_color_space(device: &ID3D11Device, hmonitor: HMONITOR) -> Option<i32> {
    let dxgi: IDXGIDevice = device.cast().ok()?;
    let adapter = unsafe { dxgi.GetAdapter() }.ok()?;
    let mut index = 0u32;
    loop {
        let output = match unsafe { adapter.EnumOutputs(index) } {
            Ok(o) => o,
            Err(_) => return None,
        };
        index += 1;
        let Ok(desc) = (unsafe { output.GetDesc() }) else {
            continue;
        };
        if desc.Monitor != hmonitor || !desc.AttachedToDesktop.as_bool() {
            continue;
        }
        let o6: IDXGIOutput6 = output.cast().ok()?;
        let d1 = unsafe { o6.GetDesc1() }.ok()?;
        return Some(d1.ColorSpace.0);
    }
}

impl DdaCapture {
    /// 为**指定显示器**创建 duplication（方案 4.1：每个需要玻璃的显示器一个
    /// 采集会话，不是每块一个）。不同分辨率/不同 DPI 的屏各自独立。
    pub fn new_for_monitor(device: &ID3D11Device, hmonitor: HMONITOR) -> Result<Self, String> {
        let dxgi: IDXGIDevice = device.cast().map_err(|e| format!("IDXGIDevice: {e}"))?;
        let adapter = unsafe { dxgi.GetAdapter() }.map_err(|e| format!("GetAdapter: {e}"))?;
        let mut index = 0u32;
        loop {
            let output = match unsafe { adapter.EnumOutputs(index) } {
                Ok(o) => o,
                Err(_) => break,
            };
            index += 1;
            let Ok(desc) = (unsafe { output.GetDesc() }) else {
                continue;
            };
            if desc.Monitor != hmonitor || !desc.AttachedToDesktop.as_bool() {
                continue;
            }
            let output1: IDXGIOutput1 = output.cast().map_err(|e| format!("IDXGIOutput1: {e}"))?;
            let duplication = unsafe { output1.DuplicateOutput(device) }
                .map_err(|e| format!("DuplicateOutput: {e}"))?;
            return Ok(Self {
                duplication,
                texture: None,
                srv: None,
                width: 0,
                height: 0,
                last_frame_signature: 0,
                acquired: 0,
                content_changed: 0,
                pointer_only: 0,
                meta: Vec::new(),
                dirty: Vec::new(),
                dirty_union: None,
                copied_px: 0,
                verify: std::env::var("GLASS_VERIFY_COPY").ok().as_deref() == Some("1"),
                verify_tex: None,
                verify_stage_a: None,
                verify_stage_b: None,
                verify_frames: 0,
                access_lost: false,
                t_dirty_us_last: 0,
                t_copy_us_last: 0,
                copy_mode: copy_mode(),
                skip_copy: std::env::var("GLASS_SKIP_COPY").ok().as_deref() == Some("1"),
                skip_dirty: std::env::var("GLASS_SKIP_DIRTY").ok().as_deref() == Some("1"),
                t_acq_us: 0,
                t_dirty_us: 0,
                t_copy_us: 0,
                t_acq_max_us: 0,
                t_pre_us: 0,
                t_rel_us: 0,
                t_rel_max_us: 0,
                hdr_warned: false,
            });
        }
        Err("该显示器上没有可用的 DXGI output（可能挂在另一块显卡上）".to_string())
    }

    /// Acquire and copy the newest desktop frame into an owned texture.
    ///
    /// `timeout_ms` 同时充当主循环的阻塞点（方案 12.4.1「以采集帧为主时钟」）：
    /// 桌面有新内容时立即返回，静止时在驱动里内核等待，CPU 占用趋近 0。
    pub fn try_capture(
        &mut self,
        device: &ID3D11Device,
        ctx: &ID3D11DeviceContext,
        timeout_ms: u32,
    ) -> Result<Option<(ID3D11Texture2D, i32, i32, bool)>, String> {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        let t0 = std::time::Instant::now();
        let acquired =
            unsafe { self.duplication.AcquireNextFrame(timeout_ms, &mut info, &mut resource) };
        let acq_us = t0.elapsed().as_micros();
        self.t_acq_us += acq_us;
        self.t_acq_max_us = self.t_acq_max_us.max(acq_us);
        if let Err(e) = acquired {
            let code = e.code().0;
            if code == DXGI_ERROR_WAIT_TIMEOUT {
                // 桌面没有新内容：正常的静默返回，调用方据此休眠。
                return Ok(None);
            }
            if code == DXGI_ERROR_ACCESS_LOST {
                self.access_lost = true;
            }
            return Err(format!("AcquireNextFrame(0x{code:08X}): {e}"));
        }
        let tp0 = std::time::Instant::now();
        let result = (|| -> Result<Option<(ID3D11Texture2D, i32, i32, bool)>, String> {
            let Some(resource) = resource else { return Ok(None); };
            let frame: ID3D11Texture2D = resource.cast().map_err(|e| format!("DDA texture: {e}"))?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            unsafe { frame.GetDesc(&mut desc); }
            let recreate = self.texture.as_ref().map_or(true, |old| {
                let mut od = D3D11_TEXTURE2D_DESC::default();
                unsafe { old.GetDesc(&mut od); }
                od.Width != desc.Width || od.Height != desc.Height || od.Format != desc.Format
            });
            if recreate {
                if desc.Format == DXGI_FORMAT_R16G16B16A16_FLOAT && !self.hdr_warned {
                    self.hdr_warned = true;
                    eprintln!(
                        "[warn] DDA 在 HDR 输出上返回 FP16 复制面：内容是 HDR 参考值，玻璃背景会过曝。                         请去掉 GLASS_BACKEND=dda 改用默认（HDR 输出会自动走 WGC）"
                    );
                }
                let owned_desc = D3D11_TEXTURE2D_DESC {
                    Width: desc.Width,
                    Height: desc.Height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: desc.Format,
                    SampleDesc: desc.SampleDesc,
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                    CPUAccessFlags: D3D11_CPU_ACCESS_FLAG(0).0 as u32,
                    MiscFlags: 0,
                };
                let mut texture = None;
                unsafe { device.CreateTexture2D(&owned_desc, None, Some(&mut texture)) }
                    .map_err(|e| format!("Create DDA texture: {e}"))?;
                let texture = texture.ok_or("Create DDA texture returned null")?;
                let mut srv = None;
                unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut srv)) }
                    .map_err(|e| format!("Create DDA SRV: {e}"))?;
                self.texture = Some(texture);
                self.srv = srv;
                self.width = desc.Width as i32;
                self.height = desc.Height as i32;
            }
            let owned = self.texture.as_ref().ok_or("DDA texture missing")?.clone();
            // 桌面内容是否真的变了（方案 12.4.1 的 NEW_BACKDROP 判据）。
            //
            // 旧实现用 `LastPresentTime` 当帧签名 —— 但 DWM 每个 vsync 都会推进
            // 该时间戳，于是「静止桌面」也被判成每帧都变，Pass A（整屏模糊，
            // 约 2.6 个全屏当量）在 110 fps 下被无谓地跑了 110 次/秒。
            // 正确判据是 duplication 自己的元数据：
            //   AccumulatedFrames      > 0  -> 累积了真实桌面更新
            //   TotalMetadataBufferSize > 0  -> 带 dirty/move rect 的内容更新
            // 两者都为 0 时只是指针形状/位置更新，桌面纹理内容未变，
            // 连 26 MB 的 CopyResource 都可以省掉（owned 纹理仍是上一帧内容）。
            let content_changed = recreate
                || info.AccumulatedFrames > 0
                || info.TotalMetadataBufferSize > 0;
            let skip_copy = self.skip_copy && !recreate;
            self.acquired += 1;
            if content_changed {
                self.content_changed += 1;
            } else {
                self.pointer_only += 1;
            }
            if content_changed {
                // 先取脏矩形，再决定拷贝范围：桌面局部更新（打字、光标闪烁、
                // 播放中的视频窗口）时只搬那几块，而不是整张 26 MB 桌面纹理。
                let td0 = std::time::Instant::now();
                let d = self.collect_dirty(&info);
                self.dirty_union = d;
                let d_us = td0.elapsed().as_micros();
                self.t_dirty_us += d_us;
                self.t_dirty_us_last = d_us;
                let tc0 = std::time::Instant::now();
                let full_area = (desc.Width as i64) * (desc.Height as i64);
                let union_area = d.map_or(i64::MAX, |r| {
                    // 先 widen 到 i64 再减：脏矩形来自 DDA 元数据（不可信），
                    // i32 直接减在 debug 构建会 overflow panic（release 回绕）。
                    (r[2] as i64 - r[0] as i64).max(0) * (r[3] as i64 - r[1] as i64).max(0)
                });
                let use_partial = !skip_copy
                    && !recreate
                    && self.copy_mode != CopyMode::Full
                    && d.is_some()
                    && !self.dirty.is_empty()
                    && (self.copy_mode == CopyMode::Union
                        || self.dirty.len() <= MAX_DIRTY_RECTS)
                    && union_area * 2 < full_area;
                if use_partial && self.copy_mode == CopyMode::Union {
                    // 只拷并集 bbox：1 次驱动调用，代价是可能搬一些没变的像素
                    let u = d.unwrap();
                    let left = u[0].clamp(0, desc.Width as i32) as u32;
                    let top = u[1].clamp(0, desc.Height as i32) as u32;
                    let right = u[2].clamp(0, desc.Width as i32) as u32;
                    let bottom = u[3].clamp(0, desc.Height as i32) as u32;
                    if right > left && bottom > top {
                        let box_ = D3D11_BOX { left, top, right, bottom, front: 0, back: 1 };
                        unsafe {
                            ctx.CopySubresourceRegion(
                                &owned, 0, left, top, 0, &frame, 0, Some(&box_),
                            );
                        }
                        self.copied_px = (right - left) as i64 * (bottom - top) as i64;
                    } else {
                        unsafe { ctx.CopyResource(&owned, &frame); }
                        self.copied_px = full_area;
                    }
                } else if use_partial {
                    let mut copied = 0i64;
                    for r in self.dirty.iter() {
                        let left = r[0].clamp(0, desc.Width as i32) as u32;
                        let top = r[1].clamp(0, desc.Height as i32) as u32;
                        let right = r[2].clamp(0, desc.Width as i32) as u32;
                        let bottom = r[3].clamp(0, desc.Height as i32) as u32;
                        if right <= left || bottom <= top {
                            continue;
                        }
                        let box_ = D3D11_BOX {
                            left,
                            top,
                            right,
                            bottom,
                            front: 0,
                            back: 1,
                        };
                        unsafe {
                            ctx.CopySubresourceRegion(
                                &owned, 0, left, top, 0, &frame, 0, Some(&box_),
                            );
                        }
                        copied += (right - left) as i64 * (bottom - top) as i64;
                    }
                    self.copied_px = copied;
                } else if !skip_copy {
                    unsafe { ctx.CopyResource(&owned, &frame); }
                    self.copied_px = full_area;
                } else {
                    self.copied_px = 0;
                }
                let c_us = tc0.elapsed().as_micros();
                self.t_copy_us += c_us;
                self.t_copy_us_last = c_us;
                if self.verify {
                    self.verify_step(device, ctx, &owned, &frame, &desc);
                }
                self.last_frame_signature =
                    self.last_frame_signature.wrapping_add(1);
            } else {
                self.dirty_union = None;
                self.copied_px = 0;
            }
            Ok(Some((owned, self.width, self.height, content_changed)))
        })();
        self.t_pre_us += tp0.elapsed().as_micros()
            .saturating_sub(self.t_dirty_us_last)
            .saturating_sub(self.t_copy_us_last);
        let tr0 = std::time::Instant::now();
        let _ = unsafe { self.duplication.ReleaseFrame() };
        let rel = tr0.elapsed().as_micros();
        self.t_rel_us += rel;
        self.t_rel_max_us = self.t_rel_max_us.max(rel);
        result
    }

    /// 参考纹理 = 每帧整屏 CopyResource；每 120 个内容帧回读比对一次。
    fn verify_step(
        &mut self,
        device: &ID3D11Device,
        ctx: &ID3D11DeviceContext,
        owned: &ID3D11Texture2D,
        frame: &ID3D11Texture2D,
        desc: &D3D11_TEXTURE2D_DESC,
    ) {
        unsafe {
            if self.verify_tex.is_none() {
                let mut ref_desc = *desc;
                ref_desc.Usage = D3D11_USAGE_DEFAULT;
                ref_desc.BindFlags = D3D11_BIND_SHADER_RESOURCE.0 as u32;
                ref_desc.CPUAccessFlags = 0;
                ref_desc.MiscFlags = 0;
                ref_desc.MipLevels = 1;
                let mut tex = None;
                if device.CreateTexture2D(&ref_desc, None, Some(&mut tex)).is_err() {
                    self.verify = false;
                    return;
                }
                self.verify_tex = tex;
                let mut stage_desc = *desc;
                stage_desc.Usage = D3D11_USAGE_STAGING;
                stage_desc.BindFlags = 0;
                stage_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
                stage_desc.MiscFlags = 0;
                stage_desc.MipLevels = 1;
                let mut sa = None;
                let mut sb = None;
                if device.CreateTexture2D(&stage_desc, None, Some(&mut sa)).is_err()
                    || device.CreateTexture2D(&stage_desc, None, Some(&mut sb)).is_err()
                {
                    self.verify = false;
                    return;
                }
                self.verify_stage_a = sa;
                self.verify_stage_b = sb;
            }
            let Some(reference) = self.verify_tex.clone() else { return };
            ctx.CopyResource(&reference, frame);
            self.verify_frames += 1;
            if self.verify_frames % 120 != 0 {
                return;
            }
            let (Some(sa), Some(sb)) =
                (self.verify_stage_a.clone(), self.verify_stage_b.clone())
            else {
                return;
            };
            ctx.CopyResource(&sa, owned);
            ctx.CopyResource(&sb, &reference);
            let mut ma = D3D11_MAPPED_SUBRESOURCE::default();
            let mut mb = D3D11_MAPPED_SUBRESOURCE::default();
            if ctx.Map(&sa, 0, D3D11_MAP_READ, 0, Some(&mut ma)).is_err() {
                return;
            }
            if ctx.Map(&sb, 0, D3D11_MAP_READ, 0, Some(&mut mb)).is_err() {
                ctx.Unmap(&sa, 0);
                return;
            }
            let row = desc.Width as usize * 4;
            let mut mismatch = 0usize;
            let mut first = None;
            for y in 0..desc.Height as usize {
                let ra = std::slice::from_raw_parts(
                    (ma.pData as *const u8).add(y * ma.RowPitch as usize),
                    row,
                );
                let rb = std::slice::from_raw_parts(
                    (mb.pData as *const u8).add(y * mb.RowPitch as usize),
                    row,
                );
                for x in (0..row).step_by(4) {
                    if ra[x] != rb[x] || ra[x + 1] != rb[x + 1] || ra[x + 2] != rb[x + 2] {
                        mismatch += 1;
                        if first.is_none() {
                            first = Some((x / 4, y));
                        }
                    }
                }
            }
            ctx.Unmap(&sa, 0);
            ctx.Unmap(&sb, 0);
            eprintln!(
                "[verify-copy] frames={} mismatch_px={} first={:?} -> {}",
                self.verify_frames,
                mismatch,
                first,
                if mismatch == 0 { "PASS" } else { "FAIL" }
            );
        }
    }

    /// 收集本帧的脏矩形（dirty + move 的源与目标），返回并集。
    /// 方案 7.4-2 / 12.5：只有真正变化的桌面区域需要重算模糊金字塔。
    fn collect_dirty(&mut self, info: &DXGI_OUTDUPL_FRAME_INFO) -> Option<[i32; 4]> {
        if self.skip_dirty {
            return None;
        }
        let size = info.TotalMetadataBufferSize;
        if size == 0 {
            return None;
        }
        if self.meta.len() < size as usize {
            self.meta.resize(size as usize, 0);
        }
        let mut union: Option<[i32; 4]> = None;
        self.dirty.clear();
        // DDA 的脏矩形/移动矩形是**不可信外部输入**：退化矩形（right<left）或
        // 垃圾坐标（±2e9）都会让后续 i32 加减在 debug 构建 overflow panic，
        // 或让 union 跨度超出 i32。统一 clamp 到屏幕并丢弃退化矩形。
        let (sw, sh) = (self.width.max(0), self.height.max(0));
        let sanitize = |r: [i32; 4]| -> Option<[i32; 4]> {
            let l = r[0].clamp(0, sw);
            let t = r[1].clamp(0, sh);
            let rr = r[2].clamp(0, sw);
            let b = r[3].clamp(0, sh);
            if rr > l && b > t {
                Some([l, t, rr, b])
            } else {
                None
            }
        };
        let ptr = self.meta.as_mut_ptr();
        unsafe {
            if let Some(sim) = sim_dirty(self.width.max(1), self.height.max(1)) {
                self.dirty = sim;
                let mut u: Option<[i32; 4]> = None;
                for r in self.dirty.iter() {
                    union_add(&mut u, *r);
                }
                return u;
            }
            let mut n: u32 = 0;
            if self
                .duplication
                .GetFrameDirtyRects(size, ptr as *mut RECT, &mut n)
                .is_ok()
            {
                for i in 0..n as usize {
                    let r = *(ptr as *const RECT).add(i);
                    let Some(rect) = sanitize([r.left, r.top, r.right, r.bottom]) else {
                        continue;
                    };
                    union_add(&mut union, rect);
                    if self.dirty.len() < MAX_DIRTY_RECTS + 1 {
                        self.dirty.push(rect);
                    }
                }
            }
            let mut m: u32 = 0;
            if self
                .duplication
                .GetFrameMoveRects(size, ptr as *mut DXGI_OUTDUPL_MOVE_RECT, &mut m)
                .is_ok()
            {
                for i in 0..m as usize {
                    let mv = *(ptr as *const DXGI_OUTDUPL_MOVE_RECT).add(i);
                    let d = mv.DestinationRect;
                    // i64 上算宽高并 clamp，避免垃圾 SourcePoint/RECT 触发溢出
                    let w = (d.right as i64 - d.left as i64).clamp(0, sw as i64);
                    let h = (d.bottom as i64 - d.top as i64).clamp(0, sh as i64);
                    let Some(dst) = sanitize([d.left, d.top, d.right, d.bottom]) else {
                        continue;
                    };
                    let sx = mv.SourcePoint.x.clamp(0, sw);
                    let sy = mv.SourcePoint.y.clamp(0, sh);
                    let Some(src) = sanitize([
                        sx,
                        sy,
                        (sx as i64 + w).min(sw as i64) as i32,
                        (sy as i64 + h).min(sh as i64) as i32,
                    ]) else {
                        continue;
                    };
                    union_add(&mut union, dst);
                    union_add(&mut union, src);
                    if self.dirty.len() < MAX_DIRTY_RECTS + 1 {
                        self.dirty.push(dst);
                        self.dirty.push(src);
                    }
                }
            }
        }
        union
    }
}
