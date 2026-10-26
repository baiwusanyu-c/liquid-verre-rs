import io, py_compile
p = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\content_layer.rs"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = """/// 把 buffer 的 [x0..x1)×[y0..y1) 子矩形拷进 GPU 纹理。
///
/// `UpdateSubresource` 的源布局语义：pSysMem 指向**整幅源图像**左上角，
/// SysMemPitch = 源行距（w*4），box 描述从源里取哪块、贴到目标哪块；
/// 2D 纹理只有一个 slice，SysMemDepthPitch 取整面大小即可。
fn upload_region(
    ctx: &ID3D11DeviceContext,
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
    unsafe {
        ctx.UpdateSubresource(
            tex,
            0,
            Some(&box_),
            buffer.as_ptr() as *const core::ffi::c_void,
            row_bytes as u32,
            (row_bytes * height as usize) as u32,
        );
    }
    Ok(())
}"""
new = """/// 把 buffer 的 [x0..x1)×[y0..y1) 子矩形拷进 GPU 纹理。
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
    let src = unsafe { buffer.as_ptr().add((y0 as usize) * row_bytes + (x0 as usize) * 4) };
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
}"""
assert old in c, "upload_region"
c = c.replace(old, new, 1)
c = c.replace("upload_region(ctx, &layer.tex, row_bytes, h, buffer, x0, y0, x1, y1)",
              "upload_region(ctx, &layer.tex, row_bytes, buffer, x0, y0, x1, y1)", 1)
c = c.replace("upload_region(ctx, &tex, row_bytes, h, buffer, 0, 0, w, h)?;",
              "upload_region(ctx, &tex, row_bytes, buffer, 0, 0, w, h)?;", 1)
# update the create/update doc bullet that mentioned完整 SUBRESOURCE_DATA
c = c.replace("""    /// - `UpdateSubresource` 传完整 `D3D11_SUBRESOURCE_DATA`（SrcDepthPitch 也
    ///   要 = 行距），或带 dirty 盒只上传子区——盒坐标是纹理像素，行距由驱动处理，
    ///   没有 Map 的 RowPitch 坑。""",
"""    /// - 带 dirty 盒只上传子区：盒坐标是纹理像素，源指针 = 子矩形左上角
    ///   （见 upload_region 的语义注释），行距由驱动处理，没有 Map 的 RowPitch 坑。""", 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
import py_compile
print("patched")
