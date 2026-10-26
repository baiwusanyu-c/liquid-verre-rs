import io
src = r'''use std::collections::HashMap;

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
    /// - 纹理是 `D3D11_USAGE_DYNAMIC`：`UpdateSubresource` 官方**不支持** DYNAMIC 资源，
    ///   plain `D3D11_MAP_WRITE` 官方**禁止**用于 DYNAMIC——两者都不是可选项，
    ///   剩下的正确组合只有 Map：
    ///   * 重建 / 全量：`WRITE_DISCARD`（驱动换分配，GPU 不读旧内容，无 hazard）；
    ///   * 同尺寸 dirty 子区：`WRITE_NO_OVERWRITE`（驱动保留旧内容，我们只 memcpy
    ///     dirty 行段；GPU 侧由驱动在 Unmap 时回收）。
    /// - `Map` 返回的 `RowPitch` 可能大于 `w*4`（驱动会 padding），逐行按 pitch 拷贝是
    ///   唯一正确姿势；这也是没选「一次大块 memcpy」的原因（坑：行距不等时大块拷贝
    ///   会把数据错位写进 padding 区）。
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
        // api 侧已校验过长度 == w*h*4；这里再防一手（命令队列的另一端是宿主直接入队时
        // 的防御性收口，debug 构建禁止整数溢出 panic，全部 checked）。
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
        let same_size = self
            .layers
            .get(&id)
            .is_some_and(|l| l.w == w && l.h == h);
        if same_size {
            let region = clip_dirty(dirty, w, h);
            if let UploadRegion::Empty = region {
                return; // 裁剪后无像素可写：连 Map 都不发
            }
            if let Some(layer) = self.layers.get(&id) {
                if let Err(e) = upload(ctx, &layer.tex, w, h, row_bytes, buffer, region) {
                    log::warn!("[content] upload failed for block {id}: {e}");
                }
            }
            return;
        }
        // 尺寸变化（或新建）：重建。dirty 在这里没有意义——新纹理没有「旧内容」可保留。
        self.layers.remove(&id);
        match create_layer(device, ctx, w, h, buffer, row_bytes) {
            Ok(layer) => {
                self.layers.insert(id, layer);
            }
            Err(e) => {
                log::warn!("[content] failed to create texture for block {id}: {e}");
            }
        }
    }

    /// 释放某块的内容纹理（显存即时归还；句柄侧 resize/隐藏前调用，FR2）。
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
    buffer: &[u8],
    row_bytes: usize,
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
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        MiscFlags: 0,
    };
    // 初始数据与全量上传同走 Map+逐行 memcpy（单一数据通路，避免两条写路径漂移）。
    let mut tex: Option<ID3D11Texture2D> = None;
    unsafe {
        device
            .CreateTexture2D(&desc, None, Some(&mut tex))
            .map_err(|e| format!("CreateTexture2D content: {e}"))?;
    }
    let tex = tex.ok_or("CreateTexture2D content returned null")?;
    upload(
        ctx,
        &tex,
        w,
        h,
        row_bytes,
        buffer,
        UploadRegion::Full,
    )?;
    let mut srv: Option<ID3D11ShaderResourceView> = None;
    unsafe {
        device
            .CreateShaderResourceView(&tex, None, Some(&mut srv))
            .map_err(|e| format!("CreateSRV content: {e}"))?;
    }
    let srv = srv.ok_or("CreateSRV content returned null")?;
    Ok(ContentLayer { tex, srv, w, h })
}

/// 把（整个或子矩形的）buffer 拷进动态纹理。`row_bytes = w*4` 已由调用方算好。
fn upload(
    ctx: &ID3D11DeviceContext,
    tex: &ID3D11Texture2D,
    w: u32,
    h: u32,
    row_bytes: usize,
    buffer: &[u8],
    region: UploadRegion,
) -> Result<()> {
    // 全量 => WRITE_DISCARD（换分配，最快）；子区 => WRITE_NO_OVERWRITE（保留其余像素）。
    let map_flag = match region {
        UploadRegion::Full => D3D11_MAP_WRITE_DISCARD,
        _ => D3D11_MAP_WRITE_NO_OVERWRITE,
    };
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe {
        ctx.Map(tex, 0, map_flag, 0, Some(&mut mapped))
            .map_err(|e| format!("Map content texture: {e}"))?;
    }
    let dst_pitch = mapped.RowPitch as usize;
    let (y0, y1, x0, x1) = match region {
        UploadRegion::Full => (0usize, h as usize, 0usize, row_bytes),
        UploadRegion::Rect { x0, y0, x1, y1 } => {
            (y0 as usize, y1 as usize, (x0 as usize) * 4, (x1 as usize) * 4)
        }
        UploadRegion::Empty => (0, 0, 0, 0),
    };
    let seg_bytes = x1.min(row_bytes) - x0.min(row_bytes);
    if seg_bytes > 0 {
        let dst = mapped.pData as *mut u8;
        for row in y0..y1.min(h as usize) {
            let src_off = row * row_bytes + x0;
            let dst_off = row * dst_pitch + x0;
            // 目标行可写字节数按 pitch 收窄（w*4 超 pitch 的畸形纹理直接截断不越界）。
            let n = seg_bytes.min(dst_pitch.saturating_sub(x0));
            unsafe {
                std::ptr::copy_nonoverlapping(buffer.as_ptr().add(src_off), dst.add(dst_off), n);
            }
        }
    }
    unsafe {
        ctx.Unmap(tex, 0);
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
'''
path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\content_layer.rs'
