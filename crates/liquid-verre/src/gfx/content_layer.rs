use std::collections::HashMap;

use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

use crate::core::content::{clip_dirty, UploadRegion};
use crate::error::Result;

/// 单块内容纹理（B8G8R8A8_UNORM，预乘 alpha，物理像素，行距 = w*4）。
pub(crate) struct ContentLayer {
    pub tex: ID3D11Texture2D,
    pub srv: ID3D11ShaderResourceView,
    pub w: u32,
    pub h: u32,
}

impl std::fmt::Debug for ContentLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContentLayer")
            .field("w", &self.w)
            .field("h", &self.h)
            .finish()
    }
}

/// block id -> 内容层。空 store 是常态（绝大多数宿主不推内容），
/// 渲染侧用 `is_empty()` 一次判断即可保证无内容帧零新增 GPU 调用（FR5）。
#[derive(Default)]
pub(crate) struct ContentStore {
    layers: HashMap<u32, ContentLayer>,
}

impl std::fmt::Debug for ContentStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContentStore")
            .field("layers", &self.layers.len())
            .finish()
    }
}

impl ContentStore {
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// 上传 / 替换某块内容。尺寸不符 => 重建纹理（旧内容随之释放，无拉伸残留）。
    ///
    /// 上传路径的选择（对照 d3d11 gotchas 清单核对过）：
    /// - 纹理用 `D3D11_USAGE_DEFAULT` + `ID3D11Device::UpdateSubresource`：
    ///   GPU 常驻内存，PS 的 `Load` 读到的一定是刚提交的数据。
    ///   （2026-10-26 bring-up 实测教训：`DYNAMIC` + `Map` 组合下 CPU staging
    ///   与 GPU 采样不同步，PS `Load` 读到的全是未初始化数据——Draw 本身
    ///   执行了、绑定全对、scissor 命中，像素却不一致；换 DEFAULT+UpdateSubresource
    ///   立即逐位正确。）
    /// - 带 dirty 盒只上传子区：盒坐标是纹理像素，源指针 = 子矩形左上角
    ///   （见 upload_region 的语义注释），行距由驱动处理，没有 Map 的 RowPitch 坑。
    /// - dirty 子区只传子矩形（FR5：dirty 窄条时避免整面上传）。
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        device: &ID3D11Device,
        ctx: &ID3D11DeviceContext,
        id: u32,
        w: u32,
        h: u32,
        buffer: &[u8],
        dirty: Option<[u32; 4]>,
    ) {
        if w == 0 || h == 0 {
            self.clear(id);
            return;
        }
        // api 侧已校验过长度 == w*h*4；这里再防一手（debug 构建禁止整数溢出
        // panic，全部 checked，坑 7）。
        let row_bytes = match (w as usize).checked_mul(4) {
            Some(n) => n,
            None => return,
        };
        let need_bytes = match (h as usize).checked_mul(row_bytes) {
            Some(n) => n,
            None => return,
        };
        if buffer.len() < need_bytes {
            log::warn!("[content] block {id}: buffer too small for {w}x{h}, skipped");
            return;
        }
        let same_size = self.layers.get(&id).is_some_and(|l| l.w == w && l.h == h);
        if same_size {
            let region = clip_dirty(dirty, w, h);
            let (x0, y0, x1, y1) = match region {
                UploadRegion::Full => (0, 0, w, h),
                UploadRegion::Rect { x0, y0, x1, y1 } => (x0, y0, x1, y1),
                UploadRegion::Empty => return, // 裁剪后无像素可写：连上传都不发
            };
            if let Some(layer) = self.layers.get(&id) {
                if let Err(e) = upload_region(ctx, &layer.tex, row_bytes, buffer, x0, y0, x1, y1) {
                    log::warn!("[content] upload failed for block {id}: {e}");
                }
            }
            return;
        }
        // 尺寸变化（或新建）：重建。dirty 在这里没有意义——新纹理没有「旧内容」可保留。
        self.layers.remove(&id);
        match create_layer(device, ctx, w, h, row_bytes, buffer) {
            Ok(layer) => {
                self.layers.insert(id, layer);
            }
            Err(e) => {
                log::warn!("[content] failed to create texture for block {id}: {e}");
            }
        }
    }

    /// 释放某块的内容纹理（显存即时归还；宿主 resize/隐藏前调用，FR2）。
    pub fn clear(&mut self, id: u32) {
        self.layers.remove(&id);
    }

    /// 全部释放（DestroyAll / 引擎退出路径）。
    pub fn clear_all(&mut self) {
        self.layers.clear();
    }

    /// 渲染侧查表（draw_blocks 交错绘制用）。
    pub fn get(&self, id: u32) -> Option<&ContentLayer> {
        self.layers.get(&id)
    }
}

fn create_layer(
    device: &ID3D11Device,
    ctx: &ID3D11DeviceContext,
    w: u32,
    h: u32,
    row_bytes: usize,
    buffer: &[u8],
) -> Result<ContentLayer> {
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
    let mut tex: Option<ID3D11Texture2D> = None;
    unsafe {
        device
            .CreateTexture2D(&desc, None, Some(&mut tex))
            .map_err(|e| format!("CreateTexture2D content: {e}"))?;
    }
    let tex = tex.ok_or("CreateTexture2D content returned null")?;
    upload_region(ctx, &tex, row_bytes, buffer, 0, 0, w, h)?;
    let mut srv: Option<ID3D11ShaderResourceView> = None;
    unsafe {
        device
            .CreateShaderResourceView(&tex, None, Some(&mut srv))
            .map_err(|e| format!("CreateSRV content: {e}"))?;
    }
    let srv = srv.ok_or("CreateSRV content returned null")?;
    Ok(ContentLayer { tex, srv, w, h })
}

#[allow(clippy::too_many_arguments)]
/// 把 buffer 的 [x0..x1)×[y0..y1) 子矩形拷进 GPU 纹理。
///
/// `UpdateSubresource` 带 pDstBox 的源布局语义（2026-10-26 实测踩坑修正）：
/// **pSrcData 指向子矩形自身的左上角**（不是整幅图左上角），SrcRowPitch =
/// 源行距（w*4）；驱动从该指针起按 box 宽高逐行取数。传整图起点会把
/// [0..w)×[0..h) 左上角的内容贴进任意 dirty 子区——verify_content 断言 4
/// 的「子区改写 39632 != 65536」抓到过这个 bug。
fn upload_region(
    ctx: &ID3D11DeviceContext,
    tex: &ID3D11Texture2D,
    row_bytes: usize,
    buffer: &[u8],
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
) -> Result<()> {
    let box_ = D3D11_BOX {
        left: x0,
        top: y0,
        front: 0,
        right: x1,
        bottom: y1,
        back: 1,
    };
    let src = unsafe {
        buffer
            .as_ptr()
            .add((y0 as usize) * row_bytes + (x0 as usize) * 4)
    };
    unsafe {
        ctx.UpdateSubresource(
            tex,
            0,
            Some(&box_),
            src as *const core::ffi::c_void,
            row_bytes as u32,
            (row_bytes * (y1 - y0) as usize) as u32,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_store_empty_by_default() {
        let store = ContentStore::default();
        assert!(store.get(1).is_none());
        assert!(store.is_empty());
        // clear / clear_all 对空 store 无害（FR2：重复调用无害）
        let mut store = ContentStore::default();
        store.clear(1);
        store.clear_all();
        assert!(store.is_empty());
    }
}
