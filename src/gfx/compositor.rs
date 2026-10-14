//! C 模式合成器：全屏画布 + 单交换链 + DComp（方案 12.1）。
//! 一次 Present、一次 Commit；块数再多也不增加交换链 / Present 次数。

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

use crate::gfx::device::Device;
use crate::gfx::glass_renderer::GlassRenderer;
use crate::gfx::pyramid::Pyramid;
use crate::gfx::swapchain::CanvasSwapchain;
use crate::gfx::DComp;
use crate::glass::manager::Manager;
use crate::glass::params::Params;

static DEBUG_SHOT_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 交换链后备缓冲：只有 RTV（flip-model 后备缓冲不允许创建 SRV）
struct BackbufferRt {
    tex: ID3D11Texture2D,
    rtv: ID3D11RenderTargetView,
    w: i32,
    h: i32,
}

pub struct Compositor {
    swap: CanvasSwapchain,
    rt: BackbufferRt,
    dcomp: DComp,
    pub pyramid: Pyramid,
    pub glass: GlassRenderer,
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
}

impl Compositor {
    pub fn new(gfx: &Device, hwnd: HWND, width: i32, height: i32) -> Result<Self, String> {
        let swap = CanvasSwapchain::new(gfx, hwnd, width, height)?;
        let rt = Self::make_backbuffer_rt(gfx, &swap, width, height)?;
        let dcomp = DComp::new(gfx, hwnd)?;
        let _visual = dcomp.attach_swapchain(&swap.swap)?;
        let pyramid = Pyramid::new(&gfx.device)?;
        let glass = GlassRenderer::new(&gfx.device)?;

        // 创建 4x4 的 fallback 纹理：在无 WGC 帧时让玻璃块仍能渲染轮廓
        let fallback_srv = Self::create_fallback_srv(gfx)?;

        Ok(Self {
            swap,
            rt,
            dcomp,
            pyramid,
            glass,
            capture_tex: None,
            capture_srv: None,
            capture_generation: 0,
            fallback_srv: Some(fallback_srv),
            width,
            height,
            background_valid: false,
            last_stage_us: [0; 4],
        })
    }

    /// 创建 fallback SRV：全屏尺寸渐变纹理，无 WGC 时也能看到漂亮的玻璃效果
    fn create_fallback_srv(gfx: &Device) -> Result<ID3D11ShaderResourceView, String> {
        // 1024×1024：看似"毛玻璃背后的桌面"的抽象图案
        let w = 1024u32;
        let h = 1024u32;
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let cx = x as f32 / w as f32;
                let cy = y as f32 / h as f32;
                // 模拟模糊桌面：大块色斑 + 柔和过渡
                let blob1 = ((cx * 3.0 + 0.5).sin() * (cy * 2.5).cos() * 0.5 + 0.5).max(0.0);
                let blob2 = ((cx * 4.5 + 2.0).cos() * (cy * 3.0 + 1.0).sin() * 0.5 + 0.5).max(0.0);
                let blob3 = ((cx * 2.0 + 1.5).sin() * (cy * 3.5).cos() * 0.5 + 0.5).max(0.0);
                // 柔和色调：蓝色/紫色/暖灰
                pixels[i]     = (100.0 + 80.0 * blob1 + 40.0 * blob2) as u8;
                pixels[i + 1] = (90.0 + 70.0 * blob2 + 50.0 * blob3) as u8;
                pixels[i + 2] = (130.0 + 60.0 * blob3 + 50.0 * blob1) as u8;
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
        Ok(())
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
    pub fn render(
        &mut self,
        gfx: &Device,
        manager: &mut Manager,
        params: &Params,
        cursor_screen: [f32; 2],
        canvas_origin: [f32; 2],
        drag_velocity: [f32; 2],
        time: f32,
        press: f32,
        hover_id: Option<u32>,
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

        // 全屏清成透明黑：flip-model 后备缓冲会保留上一帧内容，
        // 不清空会在块外留下旧像素（DComp 直接叠加到桌面）。
        unsafe {
            gfx.ctx
                .ClearRenderTargetView(&self.rt.rtv, &[0.0, 0.0, 0.0, 0.0]);
        }
        // Pass A：全屏共享模糊金字塔。
        // 关键性能约束：它与块数无关，并且静态背景时不重复执行；
        // 鼠标/拖拽只进入 Pass B，不触发整屏 down/up-sample。
        if !self.background_valid {
            let t = std::time::Instant::now();
            self.pyramid
                .run(&gfx.ctx, &capture_srv, self.width, self.height);
            self.last_stage_us[0] = t.elapsed().as_micros();
            self.background_valid = true;
        } else {
            self.last_stage_us[0] = 0;
        }
        // Pass B：逐块 scissor 绘制
        let pass_b_start = std::time::Instant::now();
        let blocks = manager.blocks_sorted();
        self.glass.draw_blocks(
            &gfx.ctx,
            &self.rt.rtv,
            self.rt.w,
            self.rt.h,
            &self.pyramid,
            &capture_srv,
            &blocks,
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
        self.last_stage_us[1] = pass_b_start.elapsed().as_micros();
        if let Ok(path) = std::env::var("GLASS_INTERNAL_SHOT") {
            if !DEBUG_SHOT_DONE.swap(true, std::sync::atomic::Ordering::AcqRel) {
                if let Err(e) = self.save_debug_bmp(gfx, &path) {
                    eprintln!("[warn] internal glass screenshot failed: {e}");
                } else {
                    eprintln!("[info] saved internal glass frame: {path}");
                }
            }
        }
        let present_start = std::time::Instant::now();
        let result = self.present();
        self.last_stage_us[2] = present_start.elapsed().as_micros();
        result
    }

    /// 保存三类诊断图：
    /// - path：最终合成预览（capture 背景 + premultiplied overlay）
    /// - path.overlay.bmp：原始 overlay backbuffer
    /// - path.capture.bmp：原始桌面捕获纹理
    /// 只有第一张与 DComp 最终视觉语义一致。
    pub fn save_debug_bmp(&self, gfx: &Device, path: &str) -> Result<(), String> {
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
                    let overlay_path = format!("{path}.overlay.bmp");
                    let capture_path = format!("{path}.capture.bmp");
                    Self::write_bmp(&overlay_path, desc.Width, desc.Height, &pixels)?;
                    Self::write_bmp(&capture_path, desc.Width, desc.Height, &capture_pixels)?;
                    Self::write_bmp(path, desc.Width, desc.Height, &composed)?;
                    if let Some(soft) = &self.pyramid.soft {
                        Self::save_texture_bmp(gfx, &soft.tex, &format!("{path}.soft.bmp"))?;
                    }
                    if let Some(frost) = &self.pyramid.frost {
                        Self::save_texture_bmp(gfx, &frost.tex, &format!("{path}.frost.bmp"))?;
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
        let present_start = std::time::Instant::now();
        self.swap.present()?;
        self.last_stage_us[3] = present_start.elapsed().as_micros();
        self.dcomp.commit()
    }
}

#[allow(dead_code)]
fn _unused(_t: ID3D11Texture2D) {
    let _ = DXGI_FORMAT_B8G8R8A8_UNORM;
}
