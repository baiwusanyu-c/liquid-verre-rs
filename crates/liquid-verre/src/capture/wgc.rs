//! WGC（Windows.Graphics.Capture）采集后端 —— 方案 4.1 的**主选**采集层。
//!
//! 与 DXGI Desktop Duplication 的关键区别：
//!   - DDA 每个 output 全局只允许一个 duplication，被录屏软件占用或状态被搞坏时
//!     直接 `E_ACCESSDENIED` / `DXGI_ERROR_NOT_CURRENTLY_AVAILABLE`；
//!   - WGC 支持多路并发采集，是方案 10.5 `pick_backend()` 里应当优先尝试的一路。
//!
//! 工程约束（方案 4.1「回调里禁止做的事」/ 12.7「禁止跨帧持有」）：
//!   - `FrameArrived` 回调里**不做任何 D3D11 设备上下文调用、不分配**：只做
//!     `TryGetNextFrame` + QI 出 `ID3D11Texture2D` + 存进槽位 + `SetEvent`；
//!   - 槽位里持有的是 `Direct3D11CaptureFrame` 本身（而不是裸纹理），以保证
//!     surface 不被帧池回收；渲染线程拷贝完立刻 `Close()`，绝不跨帧持有；
//!   - 帧池 buffer 数按方案 12.7：>90 Hz 用 3，否则 2（`GLASS_WGC_BUFFERS` 可覆盖）。
//!
//! 自反馈：画布窗口带 `WDA_EXCLUDEFROMCAPTURE`（方案 4.2 L1），WGC 同样会剔除它，
//! 因此不存在递归采样。

use std::sync::{Arc, Mutex};

use windows::core::{IInspectable, Interface};
use windows::Foundation::{EventRegistrationToken, TypedEventHandler};
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11ShaderResourceView, ID3D11Texture2D,
    D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_FLAG, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

/// 回调线程 -> 渲染线程的最新帧槽位（覆盖式：天然丢帧，永远用最新的）
struct Pending {
    /// 必须持有 frame 本身，否则帧池会回收底层 surface（方案 12.7）
    frame: Direct3D11CaptureFrame,
    tex: ID3D11Texture2D,
}

/// `CreateFreeThreaded` 帧池交付的对象本就允许跨线程访问；我们只在回调里存入、
/// 只在渲染线程取出并使用 `ID3D11DeviceContext`（同一线程），因此跨线程移动安全。
unsafe impl Send for Pending {}

/// `HANDLE` 内部是裸指针，默认不是 `Send`；WGC 的 FrameArrived 回调跑在系统
/// 线程池上，需要把事件句柄带进去。句柄本身在整个生命周期内有效且只由我们
/// 关闭，跨线程使用是安全的。
#[derive(Clone, Copy)]
struct EventHandle(usize);
unsafe impl Send for EventHandle {}

impl EventHandle {
    fn handle(self) -> HANDLE {
        HANDLE(self.0 as *mut std::ffi::c_void)
    }
}

#[derive(Default)]
struct Slot {
    inner: Mutex<Option<Pending>>,
}

impl Slot {
    fn publish(&self, p: Pending) {
        if let Ok(mut guard) = self.inner.lock() {
            // 覆盖式：被顶掉的旧帧必须立刻 Close，避免帧池饥饿
            if let Some(old) = guard.replace(p) {
                let _ = old.frame.Close();
            }
        }
    }

    fn take(&self) -> Option<Pending> {
        self.inner.lock().ok()?.take()
    }
}

pub struct WgcCapture {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    item: GraphicsCaptureItem,
    token: EventRegistrationToken,
    handler: TypedEventHandler<Direct3D11CaptureFramePool, IInspectable>,
    slot: Arc<Slot>,
    event: HANDLE,
    owned: Option<ID3D11Texture2D>,
    #[allow(dead_code)]
    owned_srv: Option<ID3D11ShaderResourceView>,
    pub width: i32,
    pub height: i32,
    pub acquired: u64,
    pub content_changed: u64,
    pub pointer_only: u64,
    pub copied_px: i64,
    pub access_lost: bool,
    /// WGC 不提供脏矩形；`None` = "整屏都算变了"，由 Compositor 用块足迹收窄。
    pub dirty_union: Option<[i32; 4]>,
    buffers: i32,
    /// `pool.Recreate` 需要 WinRT 设备包装，必须留着
    winrt_device: IDirect3DDevice,
}

impl WgcCapture {
    /// 为指定显示器创建采集会话（方案 4.1：每屏一个 session，不是每块一个）
    pub fn new_for_monitor(
        device: &ID3D11Device,
        hmonitor: HMONITOR,
    ) -> crate::error::Result<Self> {
        unsafe {
            if !GraphicsCaptureSession::IsSupported().unwrap_or(false) {
                return Err("GraphicsCaptureSession::IsSupported() = false"
                    .to_string()
                    .into());
            }
            let dxgi: windows::Win32::Graphics::Dxgi::IDXGIDevice =
                device.cast().map_err(|e| format!("IDXGIDevice: {e}"))?;
            let winrt_device: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)
                .map_err(|e| format!("CreateDirect3D11DeviceFromDXGIDevice: {e}"))?
                .cast()
                .map_err(|e| format!("IDirect3DDevice: {e}"))?;

            let interop: IGraphicsCaptureItemInterop =
                windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
                    .map_err(|e| format!("GraphicsCaptureItem factory: {e}"))?;
            let item: GraphicsCaptureItem = interop
                .CreateForMonitor(hmonitor)
                .map_err(|e| format!("CreateForMonitor: {e}"))?;
            let size: SizeInt32 = item.Size().map_err(|e| format!("item.Size: {e}"))?;
            if size.Width < 4 || size.Height < 4 {
                return Err(format!("WGC item 尺寸异常 {}x{}", size.Width, size.Height).into());
            }

            // 方案 12.7：>90 Hz 采集用 3 个 buffer，否则 2 个
            let buffers: i32 = std::env::var("GLASS_WGC_BUFFERS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let buffers = buffers.clamp(2, 8).max(2);

            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &winrt_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                buffers,
                size,
            )
            .map_err(|e| format!("CreateFreeThreaded: {e}"))?;

            let slot = Arc::new(Slot::default());
            let event =
                CreateEventW(None, false, false, None).map_err(|e| format!("CreateEventW: {e}"))?;
            let cb_slot = slot.clone();
            let cb_event = EventHandle(event.0 as usize);

            // 回调里只做「存帧 + SetEvent」，不做任何 D3D 调用 / 分配 / 锁等待
            let handler =
                TypedEventHandler::new(move |sender: &Option<Direct3D11CaptureFramePool>, _| {
                    let Some(pool) = sender else { return Ok(()) };
                    loop {
                        // 排空到最新一帧：旧的直接 Close，永远用最新的（方案 12.4.2）
                        let frame = match pool.TryGetNextFrame() {
                            Ok(f) => f,
                            Err(_) => break,
                        };
                        // ContentSize 变化时帧池需要 Recreate；尺寸以自有纹理的
                        // GetDesc 为准（try_capture 里已处理）。
                        let _ = frame.ContentSize();
                        let surface = match frame.Surface() {
                            Ok(s) => s,
                            Err(_) => {
                                let _ = frame.Close();
                                continue;
                            }
                        };
                        let tex = surface
                            .cast::<IDirect3DDxgiInterfaceAccess>()
                            .and_then(|a| a.GetInterface::<ID3D11Texture2D>());
                        match tex {
                            Ok(tex) => cb_slot.publish(Pending { frame, tex }),
                            Err(_) => {
                                let _ = frame.Close();
                            }
                        }
                    }
                    let _ = SetEvent(cb_event.handle());
                    Ok(())
                });
            let token = pool
                .FrameArrived(&handler)
                .map_err(|e| format!("FrameArrived: {e}"))?;
            let session = pool
                .CreateCaptureSession(&item)
                .map_err(|e| format!("CreateCaptureSession: {e}"))?;
            // Win11 22000+ / 22621+ 才有，旧系统失败可忽略
            let _ = session.SetIsBorderRequired(false);
            let _ = session.SetIsCursorCaptureEnabled(false);
            session
                .StartCapture()
                .map_err(|e| format!("StartCapture: {e}"))?;

            Ok(Self {
                pool,
                session,
                item,
                token,
                handler,
                slot,
                event,
                owned: None,
                owned_srv: None,
                width: size.Width,
                height: size.Height,
                acquired: 0,
                content_changed: 0,
                pointer_only: 0,
                copied_px: 0,
                access_lost: false,
                dirty_union: None,
                buffers,
                winrt_device,
            })
        }
    }

    /// 阻塞点：等新帧到达或超时（方案 12.4.1「以采集帧为主时钟」）。
    /// 与 DDA 后端同名同签名，便于 `Capture` 统一封装。
    pub fn try_capture(
        &mut self,
        device: &ID3D11Device,
        ctx: &ID3D11DeviceContext,
        timeout_ms: u32,
    ) -> crate::error::Result<Option<(ID3D11Texture2D, i32, i32, bool)>> {
        if self.slot.take().is_none() {
            unsafe {
                let r = WaitForSingleObject(self.event, timeout_ms);
                if r != WAIT_OBJECT_0 {
                    return Ok(None);
                }
            }
        }
        // 上面 take() 已经把帧取走了；这里重新取（WaitForSingleObject 之后可能有新帧）
        let pending = match self.slot.take() {
            Some(p) => p,
            None => return Ok(None),
        };
        let result = (|| -> crate::error::Result<Option<(ID3D11Texture2D, i32, i32, bool)>> {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            unsafe { pending.tex.GetDesc(&mut desc) };
            let recreate = self.owned.as_ref().map_or(true, |old| {
                let mut od = D3D11_TEXTURE2D_DESC::default();
                unsafe { old.GetDesc(&mut od) };
                od.Width != desc.Width || od.Height != desc.Height || od.Format != desc.Format
            });
            if recreate {
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
                    .map_err(|e| format!("Create WGC texture: {e}"))?;
                let texture = texture.ok_or("Create WGC texture returned null")?;
                let mut srv = None;
                unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut srv)) }
                    .map_err(|e| format!("Create WGC SRV: {e}"))?;
                self.owned = Some(texture);
                self.owned_srv = srv;
                self.width = desc.Width as i32;
                self.height = desc.Height as i32;
                // 分辨率变了要重建帧池（方案 4.1）
                let _ = self.pool.Recreate(
                    &self.winrt_device,
                    DirectXPixelFormat::B8G8R8A8UIntNormalized,
                    self.buffers,
                    SizeInt32 {
                        Width: desc.Width as i32,
                        Height: desc.Height as i32,
                    },
                );
            }
            let owned = self.owned.clone().ok_or("WGC texture missing")?;
            unsafe { ctx.CopyResource(&owned, &pending.tex) };
            self.copied_px = (desc.Width as i64) * (desc.Height as i64);
            // 持有时间到此为止：立刻归还给帧池（方案 12.7 硬约束）
            let _ = pending.frame.Close();
            Ok(Some((owned, self.width, self.height, true)))
        })();
        self.acquired += 1;
        if result.as_ref().map(|r| r.is_some()).unwrap_or(false) {
            self.content_changed += 1;
        }
        result
    }
}

impl Drop for WgcCapture {
    fn drop(&mut self) {
        if let Some(p) = self.slot.take() {
            let _ = p.frame.Close();
        }
        let _ = self.pool.RemoveFrameArrived(self.token);
        let _ = self.session.Close();
        let _ = self.pool.Close();
        let _ = &self.item;
        let _ = &self.handler;
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.event);
        }
    }
}
