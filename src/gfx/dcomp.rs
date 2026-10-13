//! DirectComposition：一次 Commit 提交全部 visual（方案 3.1 / 5.1 Pass C）。

use windows::core::Interface;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGISwapChain1};

use crate::gfx::device::Device;

pub struct DComp {
    pub device: IDCompositionDevice,
    _target: IDCompositionTarget,
    root: IDCompositionVisual,
}

impl DComp {
    pub fn new(gfx: &Device, hwnd: HWND) -> Result<Self, String> {
        let dxgi: IDXGIDevice = gfx.device.cast().map_err(|e| format!("IDXGIDevice: {e}"))?;
        let device: IDCompositionDevice = unsafe { DCompositionCreateDevice(&dxgi) }
            .map_err(|e| format!("DCompositionCreateDevice: {e}"))?;
        let target = unsafe { device.CreateTargetForHwnd(hwnd, true) }
            .map_err(|e| format!("CreateTargetForHwnd: {e}"))?;
        let root = unsafe { device.CreateVisual() }.map_err(|e| format!("CreateVisual: {e}"))?;
        unsafe {
            target.SetRoot(&root).map_err(|e| format!("SetRoot: {e}"))?;
            device.Commit().map_err(|e| format!("Commit: {e}"))?;
        }
        Ok(Self {
            device,
            _target: target,
            root,
        })
    }

    /// 为一个交换链创建 visual，插到根下面。
    pub fn attach_swapchain(&self, swap: &IDXGISwapChain1) -> Result<IDCompositionVisual, String> {
        let visual = unsafe { self.device.CreateVisual() }.map_err(|e| e.to_string())?;
        unsafe {
            visual
                .SetContent(swap)
                .map_err(|e| format!("visual.SetContent: {e}"))?;
            self.root
                .AddVisual(&visual, true, None)
                .map_err(|e| format!("AddVisual: {e}"))?;
        }
        Ok(visual)
    }

    pub fn commit(&self) -> Result<(), String> {
        unsafe {
            self.device
                .Commit()
                .map_err(|e| format!("DComp Commit: {e}"))
        }
    }
}
