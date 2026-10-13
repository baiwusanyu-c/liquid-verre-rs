//! DXGI Desktop Duplication capture.
//!
//! Unlike monitor WGC + WDA_EXCLUDEFROMCAPTURE, duplication does not replace an
//! excluded full-screen overlay with a black rectangle. The render thread copies
//! the acquired frame into an owned SRV texture before ReleaseFrame.

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11ShaderResourceView, ID3D11Texture2D,
    D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_FLAG, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIDevice, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
    DXGI_OUTDUPL_FRAME_INFO,
};

pub struct DdaCapture {
    duplication: IDXGIOutputDuplication,
    texture: Option<ID3D11Texture2D>,
    srv: Option<ID3D11ShaderResourceView>,
    pub width: i32,
    pub height: i32,
}

impl DdaCapture {
    pub fn new(device: &ID3D11Device) -> Result<Self, String> {
        let dxgi: IDXGIDevice = device.cast().map_err(|e| format!("IDXGIDevice: {e}"))?;
        let adapter = unsafe { dxgi.GetAdapter() }.map_err(|e| format!("GetAdapter: {e}"))?;
        let output = unsafe { adapter.EnumOutputs(0) }.map_err(|e| format!("EnumOutputs: {e}"))?;
        let output1: IDXGIOutput1 = output.cast().map_err(|e| format!("IDXGIOutput1: {e}"))?;
        let duplication = unsafe { output1.DuplicateOutput(device) }
            .map_err(|e| format!("DuplicateOutput: {e}"))?;
        Ok(Self { duplication, texture: None, srv: None, width: 0, height: 0 })
    }

    /// Acquire and copy the newest desktop frame into an owned texture.
    pub fn try_capture(
        &mut self,
        device: &ID3D11Device,
        ctx: &ID3D11DeviceContext,
    ) -> Result<Option<(ID3D11Texture2D, i32, i32)>, String> {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        let acquired = unsafe { self.duplication.AcquireNextFrame(0, &mut info, &mut resource) };
        if acquired.is_err() {
            return Ok(None);
        }
        let result = (|| -> Result<Option<(ID3D11Texture2D, i32, i32)>, String> {
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
            let owned = self.texture.as_ref().ok_or("DDA texture missing")?;
            unsafe { ctx.CopyResource(owned, &frame); }
            Ok(Some((owned.clone(), self.width, self.height)))
        })();
        let _ = unsafe { self.duplication.ReleaseFrame() };
        result
    }
}
