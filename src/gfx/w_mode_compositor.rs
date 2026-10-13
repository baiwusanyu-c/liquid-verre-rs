//! W 模式合成器：每块独立窗口 + 独立交换链（方案 12.1 W 模式）
//! 适用于块数 ≤ 32 且总面积 < 1×屏幕的场景

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIFactory2, IDXGISwapChain1, DXGI_SCALING_STRETCH,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
    DXGI_CREATE_FACTORY_FLAGS, DXGI_PRESENT, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};

use crate::gfx::device::Device;
use crate::gfx::shaders::{make_rt, RenderTarget};
use crate::glass::block::Block;

/// 单个玻璃块的渲染资源
pub struct BlockRenderer {
    pub hwnd: HWND,
    pub swapchain: IDXGISwapChain1,
    pub rtv: ID3D11RenderTargetView,
    pub width: i32,
    pub height: i32,
    pub visual: windows::Win32::Graphics::DirectComposition::IDCompositionVisual,
}

impl BlockRenderer {
    /// 为单个块创建交换链和 DComp Visual
    pub fn new(gfx: &Device, hwnd: HWND, width: i32, height: i32) -> Result<Self, String> {
        let factory: IDXGIFactory2 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) }
            .map_err(|e| format!("CreateDXGIFactory2: {e}"))?;

        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width.max(1) as u32,
            Height: height.max(1) as u32,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: DXGI_SWAP_CHAIN_FLAG(0), // W 模式不需要 waitable object
        };

        let swapchain = unsafe { factory.CreateSwapChainForComposition(&gfx.device, &desc, None) }
            .map_err(|e| format!("CreateSwapChainForComposition: {e}"))?;

        // 创建 DComp Visual
        let dcomp = &gfx.dcomp;
        let target = unsafe { dcomp.CreateTargetForHwnd(hwnd, true) }
            .map_err(|e| format!("CreateTargetForHwnd: {e}"))?;
        let visual = unsafe { dcomp.CreateVisual() }
            .map_err(|e| format!("CreateVisual: {e}"))?;
        unsafe {
            visual.SetContent(&swapchain).map_err(|e| format!("SetContent: {e}"))?;
            target.SetRoot(&visual).map_err(|e| format!("SetRoot: {e}"))?;
        }

        let rtv = {
            let tex: ID3D11Texture2D = unsafe { swapchain.GetBuffer(0) }
                .map_err(|e| format!("GetBuffer: {e}"))?;
            let mut rtv = None;
            unsafe {
                gfx.device.CreateRenderTargetView(&tex, None, Some(&mut rtv))
                    .map_err(|e| format!("CreateRenderTargetView: {e}"))?;
            }
            rtv.ok_or("CreateRenderTargetView returned None")?
        };

        Ok(Self {
            hwnd,
            swapchain,
            rtv,
            width,
            height,
            visual,
        })
    }

    /// 调整大小（零停顿：只改交换链，不重建窗口）
    pub fn resize(&mut self, gfx: &Device, width: i32, height: i32) -> Result<(), String> {
        if self.width == width && self.height == height {
            return Ok(());
        }

        unsafe {
            self.swapchain.ResizeBuffers(
                0,
                width.max(1) as u32,
                height.max(1) as u32,
                DXGI_FORMAT_R8G8B8A8_UNORM,
                DXGI_SWAP_CHAIN_FLAG(0),
            ).map_err(|e| format!("ResizeBuffers: {e}"))?;

            // 重新创建 RTV
            let tex: ID3D11Texture2D = self.swapchain.GetBuffer(0)
                .map_err(|e| format!("GetBuffer after resize: {e}"))?;
            let mut rtv = None;
            gfx.device.CreateRenderTargetView(&tex, None, Some(&mut rtv))
                .map_err(|e| format!("CreateRenderTargetView after resize: {e}"))?;
            self.rtv = rtv.ok_or("CreateRenderTargetView after resize returned None")?;
        }

        self.width = width;
        self.height = height;
        Ok(())
    }

    pub fn present(&self) -> Result<(), String> {
        unsafe {
            self.swapchain.Present(1, DXGI_PRESENT(0))
                .ok().map_err(|e| format!("Present: {e}"))?;
        }
        Ok(())
    }
}

/// W 模式管理器：管理多个独立块
pub struct WModeCompositor {
    pub device: Device,
    pub blocks: Vec<BlockRenderer>,
}

impl WModeCompositor {
    pub fn new(device: Device) -> Self {
        Self {
            device,
            blocks: Vec::new(),
        }
    }

    /// 添加一个块
    pub fn add_block(&mut self, hwnd: HWND, width: i32, height: i32) -> Result<usize, String> {
        let block = BlockRenderer::new(&self.device, hwnd, width, height)?;
        self.blocks.push(block);
        Ok(self.blocks.len() - 1)
    }

    /// 移除一个块
    pub fn remove_block(&mut self, index: usize) {
        if index < self.blocks.len() {
            self.blocks.remove(index);
        }
    }

    /// 清空所有块
    pub fn clear(&mut self) {
        self.blocks.clear();
    }

    /// 渲染所有块
    pub fn render_all(&self, pyramid: &crate::gfx::pyramid::Pyramid) -> Result<(), String> {
        for block in &self.blocks {
            block.present()?;
        }
        // 统一提交 DComp
        unsafe {
            self.device.dcomp.Commit().map_err(|e| format!("DComp Commit: {e}"))?;
        }
        Ok(())
    }
}