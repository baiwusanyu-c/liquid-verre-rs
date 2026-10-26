import io
p = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\content_layer.rs"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = """    let sub = D3D11_SUBRESOURCE_DATA {
        pSysMem: buffer.as_ptr() as *const _,
        SysMemPitch: row_bytes as u32,
        SysMemSlicePitch: (row_bytes * height as usize) as u32,
    };
    unsafe {
        device.UpdateSubresource(Some(tex), 0, Some(&box_), &sub);
    }
    Ok(())"""
if old not in c:
    old = None
    i = c.find("let sub = D3D11_SUBRESOURCE_DATA")
    j = c.find("Ok(())", i)
    print(repr(c[i:j+7]))
    raise SystemExit("print actual")
