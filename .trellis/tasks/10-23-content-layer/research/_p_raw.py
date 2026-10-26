import io
p = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\content_layer.rs"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = """    let sub = D3D11_SUBRESOURCE_DATA {
        pSysMem: buffer.as_ptr() as *const _,
        SysMemPitch: row_bytes as u32,
        SysMemDepthPitch: (row_bytes * height as usize) as u32,
    };
    unsafe {
        device.UpdateSubresource(Some(tex), 0, Some(&box_), &sub);
    }
    Ok(())"""
new = """    unsafe {
        device.UpdateSubresource(
            tex,
            0,
            Some(&box_),
            buffer.as_ptr() as *const core::ffi::c_void,
            row_bytes as u32,
            (row_bytes * height as usize) as u32,
        );
    }
    Ok(())"""
assert old in c
c = c.replace(old, new, 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("raw UpdateSubresource call")
