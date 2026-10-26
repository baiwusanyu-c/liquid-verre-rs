import io

# Fix 1: content_layer.rs - fix imports and create_layer signature
path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\content_layer.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()

# Fix imports
old_imports = "use windows::Win32::Graphics::Direct3D11::*;"
new_imports = """use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};"""
assert old_imports in content, "imports not found"
content = content.replace(old_imports, new_imports, 1)

# Fix create_layer - add ctx parameter
old_create = "    fn create_layer(\n        device: &ID3D11Device,\n        w: u32,\n        h: u32,\n        buffer: &[u8],\n    ) -> Result<ContentLayer> {"
new_create = "    fn create_layer(\n        device: &ID3D11Device,\n        ctx: &ID3D11DeviceContext,\n        w: u32,\n        h: u32,\n        buffer: &[u8],\n    ) -> Result<ContentLayer> {"
assert old_create in content, "create_layer sig not found"
content = content.replace(old_create, new_create, 1)

# Fix the call site in update
old_call = "match Self::create_layer(device, w, h, buffer) {"
new_call = "match Self::create_layer(device, ctx, w, h, buffer) {"
assert old_call in content, "create_layer call not found"
content = content.replace(old_call, new_call, 1)

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("content_layer.rs: fixed imports, create_layer signature, call site")

# Fix 2: app.rs - use full path for ContentStore
path2 = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\runtime\app.rs"
with io.open(path2, "r", encoding="utf-8") as f:
    content2 = f.read()
# Remove the old wrong import
content2 = content2.replace("\nuse crate::gfx::ContentStore;\n", "\n")
# Fix field type
old_field2 = "    pub content_store: ContentStore,"
new_field2 = "    pub content_store: crate::gfx::ContentStore,"
assert old_field2 in content2, "content_store field not found"
content2 = content2.replace(old_field2, new_field2, 1)
with io.open(path2, "w", encoding="utf-8", newline="\n") as f:
    f.write(content2)
print("app.rs: fixed ContentStore type")

# Fix 3: mod.rs - use full path for ContentStore init
path3 = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\runtime\mod.rs"
with io.open(path3, "r", encoding="utf-8") as f:
    content3 = f.read()
# Remove the old import
content3 = content3.replace("use crate::gfx::{ContentStore, Device, GlassRenderer};", "use crate::gfx::{Device, GlassRenderer};")
old_init3 = "        content_store: ContentStore::default(),"
new_init3 = "        content_store: crate::gfx::ContentStore::default(),"
assert old_init3 in content3, "content_store init not found"
content3 = content3.replace(old_init3, new_init3, 1)
with io.open(path3, "w", encoding="utf-8", newline="\n") as f:
    f.write(content3)
print("mod.rs: fixed ContentStore init")

# Fix 4: commands.rs - fix monitor_index
path4 = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\runtime\commands.rs"
with io.open(path4, "r", encoding="utf-8") as f:
    content4 = f.read()
old_idx = "                let mi = b.monitor_index;"
new_idx = "                let mi = b.monitor();"
assert old_idx in content4, "monitor_index not found"
content4 = content4.replace(old_idx, new_idx, 1)
with io.open(path4, "w", encoding="utf-8", newline="\n") as f:
    f.write(content4)
print("commands.rs: fixed monitor_index -> monitor()")
