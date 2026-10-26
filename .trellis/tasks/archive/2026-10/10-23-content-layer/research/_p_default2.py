import io, os
R = r"D:\github-project\liquid-verre-rs"
p = os.path.join(R, r"crates\liquid-verre\src\gfx\content_layer.rs")
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()

old = """/// 把 buffer 的 [x0..x1)×[y0..y1) 子矩形拷进 GPU 纹理。
fn upload_region(
    device: &ID3D11Device,
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
    let sub = D3D11_SUBRESOURCE_DATA {
        pSysMem: buffer.as_ptr() as *const _,
        SysMemPitch: row_bytes as u32,
        // 子矩形上传时源仍是完整图像，深度方向一行 = 整面（2D 纹理只有一层）。
        SysMemDepthPitch: row_bytes as u32 * (y1 - y0).max(1) as u32 * (y1 > y0) as u32
            + (y1 == y0) as u32,
    };
    unsafe {
        device.UpdateSubresource(tex, 0, Some(&box_), &sub);
    }
    Ok(())
}"""
new = """/// 把 buffer 的 [x0..x1)×[y0..y1) 子矩形拷进 GPU 纹理。
///
/// `UpdateSubresource` 的源布局语义：pSysMem 指向**整幅源图像**左上角，
/// SysMemPitch = 源行距（w*4），box 描述从源里取哪块、贴到目标哪块；
/// 2D 纹理只有一个 slice，SysMemDepthPitch 取整面大小即可。
fn upload_region(
    device: &ID3D11Device,
    tex: &ID3D11Texture2D,
    row_bytes: usize,
    height: u32,
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
    let sub = D3D11_SUBRESOURCE_DATA {
        pSysMem: buffer.as_ptr() as *const _,
        SysMemPitch: row_bytes as u32,
        SysMemDepthPitch: (row_bytes * height as usize) as u32,
    };
    unsafe {
        device.UpdateSubresource(Some(tex), 0, Some(&box_), &sub);
    }
    Ok(())
}"""
assert old in c
c = c.replace(old, new, 1)

c = c.replace(
    "if let Err(e) = upload_region(device, &layer.tex, row_bytes, buffer, x0, y0, x1, y1) {",
    "if let Err(e) = upload_region(device, &layer.tex, row_bytes, h, buffer, x0, y0, x1, y1) {",
    1,
)
c = c.replace(
    "upload_region(device, &tex, row_bytes, buffer, 0, 0, w, h)?;",
    "upload_region(device, &tex, row_bytes, h, buffer, 0, 0, w, h)?;",
    1,
)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("upload_region fixed")
