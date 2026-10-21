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
    D3D11_CPU_ACCESS_FLAG, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
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
}
