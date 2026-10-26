import io
p = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\content_layer.rs"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
# use ctx.UpdateSubresource instead of device
c = c.replace("    pub fn update(\n        &mut self,\n        device: &ID3D11Device,\n        _ctx: &ID3D11DeviceContext,",
              "    pub fn update(\n        &mut self,\n        device: &ID3D11Device,\n        ctx: &ID3D11DeviceContext,", 1)
c = c.replace("if let Err(e) = upload_region(device, &layer.tex, row_bytes, h, buffer, x0, y0, x1, y1) {",
              "if let Err(e) = upload_region(ctx, &layer.tex, row_bytes, h, buffer, x0, y0, x1, y1) {", 1)
c = c.replace("match create_layer(device, w, h, row_bytes, buffer) {",
              "match create_layer(device, ctx, w, h, row_bytes, buffer) {", 1)
c = c.replace("""fn create_layer(
    device: &ID3D11Device,
    w: u32,""",
"""fn create_layer(
    device: &ID3D11Device,
    ctx: &ID3D11DeviceContext,
    w: u32,""", 1)
c = c.replace("upload_region(device, &tex, row_bytes, h, buffer, 0, 0, w, h)?;",
              "upload_region(ctx, &tex, row_bytes, h, buffer, 0, 0, w, h)?;", 1)
c = c.replace("""fn upload_region(
    device: &ID3D11Device,
    tex: &ID3D11Texture2D,""",
"""fn upload_region(
    ctx: &ID3D11DeviceContext,
    tex: &ID3D11Texture2D,""", 1)
c = c.replace("""    unsafe {
        device.UpdateSubresource(
            tex,
            0,
            Some(&box_),
            buffer.as_ptr() as *const core::ffi::c_void,
            row_bytes as u32,
            (row_bytes * height as usize) as u32,
        );
    }""",
"""    unsafe {
        ctx.UpdateSubresource(
            tex,
            0,
            Some(&box_),
            buffer.as_ptr() as *const core::ffi::c_void,
            row_bytes as u32,
            (row_bytes * height as usize) as u32,
        );
    }""", 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("ctx.UpdateSubresource wired")
