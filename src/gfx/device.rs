//! D3D11 设备与 DXGI 工厂（方案 3.4）：单设备 + 单立即上下文。

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL,
    D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGIFactory2};

pub struct Device {
    pub device: ID3D11Device,
    pub ctx: ID3D11DeviceContext,
    pub factory: IDXGIFactory2,
    pub feature_level: D3D_FEATURE_LEVEL,
    pub warp: bool,
}

impl Device {
    /// 优先硬件，失败回落 WARP（方案 8-R11）。
    pub fn create() -> Result<Self, String> {
        match Self::create_with(D3D_DRIVER_TYPE_HARDWARE) {
            Ok(d) => Ok(d),
            Err(hw_err) => Self::create_with(D3D_DRIVER_TYPE_WARP).map_err(|warp_err| {
                format!("硬件 D3D11 失败（{hw_err}），WARP 也失败（{warp_err}）")
            }),
        }
    }

    fn create_with(driver: D3D_DRIVER_TYPE) -> Result<Self, String> {
        let levels = [D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_0];
        let mut device = None;
        let mut ctx = None;
        let mut level = D3D_FEATURE_LEVEL::default();
        unsafe {
            D3D11CreateDevice(
                None,
                driver,
                None,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                Some(&mut level),
                Some(&mut ctx),
            )
            .map_err(|e| format!("D3D11CreateDevice: {e}"))?;
        }
        let device = device.ok_or("D3D11CreateDevice 未返回设备")?;
        let ctx = ctx.ok_or("D3D11CreateDevice 未返回上下文")?;
        let dxgi: IDXGIDevice = device.cast().map_err(|e| format!("IDXGIDevice: {e}"))?;
        let adapter = unsafe { dxgi.GetAdapter() }.map_err(|e| format!("GetAdapter: {e}"))?;
        let factory: IDXGIFactory2 =
            unsafe { adapter.GetParent() }.map_err(|e| format!("IDXGIFactory2: {e}"))?;
        Ok(Self {
            device,
            ctx,
            factory,
            feature_level: level,
            warp: driver == D3D_DRIVER_TYPE_WARP,
        })
    }

    /// 是否支持 SM 4.1+（Feature Level 10.0 即可跑本方案）
    pub fn supports_shader_model_41(&self) -> bool {
        self.feature_level.0 >= D3D_FEATURE_LEVEL_10_0.0
    }
}
