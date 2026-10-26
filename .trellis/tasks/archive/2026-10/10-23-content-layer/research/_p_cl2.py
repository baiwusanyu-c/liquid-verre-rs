import io
path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\content_layer.rs"
with io.open(path, "r", encoding="utf-8") as f:
    c = f.read()
# upload(): drop unused w param
c = c.replace("""fn upload(
    ctx: &ID3D11DeviceContext,
    tex: &ID3D11Texture2D,
    w: u32,
    h: u32,""", """fn upload(
    ctx: &ID3D11DeviceContext,
    tex: &ID3D11Texture2D,
    h: u32,""", 1)
c = c.replace("upload(ctx, &layer.tex, w, h, row_bytes, buffer, region)", "upload(ctx, &layer.tex, h, row_bytes, buffer, region)", 1)
c = c.replace("upload(ctx, &tex, w, h, row_bytes, buffer, UploadRegion::Full)", "upload(ctx, &tex, h, row_bytes, buffer, UploadRegion::Full)", 1)
with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("upload param cleaned")
