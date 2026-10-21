//! CreateSwapChainForComposition + 预乘 alpha（方案 3.5 / 5.9）。

use windows::Win32::Foundation::{BOOL, HWND};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIFactory2, IDXGISwapChain1, DXGI_CREATE_FACTORY_FLAGS, DXGI_PRESENT,
    DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};

use crate::gfx::device::Device;

pub struct CanvasSwapchain {
    pub swap: IDXGISwapChain1,
    pub width: i32,
    pub height: i32,
}

impl CanvasSwapchain {
    pub fn new(gfx: &Device, _hwnd: HWND, width: i32, height: i32) -> crate::error::Result<Self> {
        let factory: IDXGIFactory2 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) }
            .map_err(|e| format!("CreateDXGIFactory2: {e}"))?;
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width.max(1) as u32,
            Height: height.max(1) as u32,
            // WGC 交付 BGRA8；DComp 合成链也使用 BGRA8，避免通道/alpha 语义转换。
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: BOOL(0),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: 0,
        };
        let swap = unsafe { factory.CreateSwapChainForComposition(&gfx.device, &desc, None) }
            .map_err(|e| format!("CreateSwapChainForComposition: {e}"))?;
        Ok(Self {
            swap,
            width,
            height,
        })
    }

    pub fn present(&self) -> crate::error::Result<()> {
        let hr = unsafe { self.swap.Present(1, DXGI_PRESENT(0)) };
        hr.ok()
            .map_err(|e| crate::error::Error::from(format!("Present: {e}")))
    }

    pub fn resize(&mut self, _gfx: &Device, width: i32, height: i32) -> crate::error::Result<()> {
        if self.width == width && self.height == height {
            return Ok(());
        }
        unsafe {
            self.swap
                .ResizeBuffers(
                    0,
                    width.max(1) as u32,
                    height.max(1) as u32,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_SWAP_CHAIN_FLAG(0),
                )
                .map_err(|e| format!("ResizeBuffers: {e}"))?;
        }
        self.width = width;
        self.height = height;
        Ok(())
    }

    pub fn backbuffer(
        &self,
        _device: &windows::Win32::Graphics::Direct3D11::ID3D11Device,
    ) -> crate::error::Result<windows::Win32::Graphics::Direct3D11::ID3D11Texture2D> {
        unsafe {
            self.swap
                .GetBuffer(0)
                .map_err(|e| crate::error::Error::from(format!("GetBuffer: {e}")))
        }
    }
}
