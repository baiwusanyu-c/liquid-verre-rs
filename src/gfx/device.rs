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
use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGIDevice1, IDXGIFactory2};

#[allow(dead_code)] // factory / feature_level 供后续后端选择与诊断使用
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
        // 方案 7.4-7：渲染队列深度压到 1 帧，光标到画面滞后最小 —— **默认值**。
        // 历史：2026-10-20 曾把默认改成 2（轮转 A/B：20 块拖拽 73.9 -> 105.0 fps、
        // p50 13.0 -> 8.34 ms），但用户实测反馈 lat=2 的 +1 帧队列（约 +8 ms 光标到
        // 光子）在快速拖拽时表现为「玻璃跟在光标后面」的拖尾感，比 lat=1 的
        // 偶发丢帧更像卡顿。于是默认回到 1（跟手优先），吞吐档保留为
        // GLASS_FRAME_LATENCY=2（适合静态展示/录屏/不跟手的场景）。
        let latency: u32 = std::env::var("GLASS_FRAME_LATENCY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1)
            .clamp(1, 8);
        eprintln!("[info] 渲染队列深度 = {latency}（GLASS_FRAME_LATENCY 可改；1 = 最低滞后/默认，2 = 吞吐优先）");
        if let Ok(dxgi1) = dxgi.cast::<IDXGIDevice1>() {
            if let Err(e) = unsafe { dxgi1.SetMaximumFrameLatency(latency) } {
                eprintln!("[warn] SetMaximumFrameLatency({latency}): {e}");
            }
        }
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

}
