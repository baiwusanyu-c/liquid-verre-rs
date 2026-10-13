//! Windows.Graphics.Capture 自由线程采集（方案 4.1）。
//!
//! 关键点：
//! - 用 CreateFreeThreaded，帧回调在系统线程池，不占用渲染线程；
//! - 回调里只做一件事：把 ID3D11Texture2D 塞进 FrameSlot，绝不做 GPU 工作；
//! - 我们的窗口已用 WDA_EXCLUDEFROMCAPTURE 排除，避免自反馈。

use std::sync::Arc;

use windows::core::{IInspectable, Interface};
use windows::Foundation::{EventRegistrationToken, TypedEventHandler};
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D};
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

use crate::capture::slot::FrameSlot;

pub struct WgcCapture {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    item: GraphicsCaptureItem,
    frame_arrived: TypedEventHandler<Direct3D11CaptureFramePool, IInspectable>,
    frame_arrived_token: EventRegistrationToken,
    pub width: i32,
    pub height: i32,
}

impl WgcCapture {
    /// 采集主显示器
    pub fn new_monitor(
        device: &ID3D11Device,
        monitor_point: POINT,
        slot: Arc<FrameSlot>,
    ) -> Result<Self, String> {
        unsafe {
            let dxgi: windows::Win32::Graphics::Dxgi::IDXGIDevice =
                device.cast().map_err(|e| format!("IDXGIDevice: {e}"))?;
            let inspect = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)
                .map_err(|e| format!("CreateDirect3D11DeviceFromDXGIDevice: {e}"))?;
            let winrt_device: IDirect3DDevice = inspect
                .cast()
                .map_err(|e| format!("IDirect3DDevice: {e}"))?;

            let interop: IGraphicsCaptureItemInterop =
                windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
                    .map_err(|e| format!("GraphicsCaptureItem factory: {e}"))?;
            let mon = MonitorFromPoint(monitor_point, MONITOR_DEFAULTTOPRIMARY);
            let item: GraphicsCaptureItem = interop
                .CreateForMonitor(mon)
                .map_err(|e| format!("CreateForMonitor: {e}"))?;
            let size: SizeInt32 = item.Size().map_err(|e| format!("item.Size: {e}"))?;

            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &winrt_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                size,
            )
            .map_err(|e| format!("CreateFreeThreaded: {e}"))?;
            let callback_slot = slot.clone();
            let frame_arrived =
                TypedEventHandler::new(move |sender: &Option<Direct3D11CaptureFramePool>, _| {
                    let Some(pool) = sender else {
                        return Ok(());
                    };

                    // TryGetNextFrame is intentionally drained only from FrameArrived. Some
                    // WGC runtimes return S_OK with an empty frame while the pool is settling;
                    // that is a normal no-frame condition, not an error to log.
                    if let Ok(frame) = pool.TryGetNextFrame() {
                        if let Ok(surface) = frame.Surface() {
                            if let Ok(access) = surface.cast::<IDirect3DDxgiInterfaceAccess>() {
                                if let Ok(tex) = access.GetInterface::<ID3D11Texture2D>() {
                                    callback_slot.publish(tex, size.Width, size.Height);
                                }
                            }
                        }
                    }
                    Ok(())
                });
            let frame_arrived_token = pool
                .FrameArrived(&frame_arrived)
                .map_err(|e| format!("FrameArrived: {e}"))?;
            let session = pool
                .CreateCaptureSession(&item)
                .map_err(|e| format!("CreateCaptureSession: {e}"))?;
            let _ = session.SetIsBorderRequired(false);
            let _ = session.SetIsCursorCaptureEnabled(false);
            session
                .StartCapture()
                .map_err(|e| format!("StartCapture: {e}"))?;

            Ok(Self {
                pool,
                session,
                item,
                frame_arrived,
                frame_arrived_token,
                width: size.Width,
                height: size.Height,
            })
        }
    }

    pub fn is_supported() -> bool {
        // GraphicsCaptureSession::IsSupported() 在 Win10 1809+ 恒为 true；
        // 真正的失败点会以 Err 形式从 new_monitor 冒出来，由调用方回退。
        true
    }
}

impl Drop for WgcCapture {
    fn drop(&mut self) {
        let _ = self.pool.RemoveFrameArrived(self.frame_arrived_token);
        let _ = self.session.Close();
        let _ = self.pool.Close();
        let _ = &self.item;
        let _ = &self.frame_arrived;
    }
}
