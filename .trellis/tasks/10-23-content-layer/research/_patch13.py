import io

path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\glass_renderer.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()

# Patch 1: Add content shader fields after border fields
old_fields = "    bcb: ID3D11Buffer,\n    /// env ema"
new_fields = "    bcb: ID3D11Buffer,\n    /// content layer overlay shaders (independent pass, like border)\n    cvs: ID3D11VertexShader,\n    cps: ID3D11PixelShader,\n    ccb: ID3D11Buffer,\n    csampler: ID3D11SamplerState,\n    /// env ema"

assert old_fields in content, "fields not found"
if new_fields not in content:
    content = content.replace(old_fields, new_fields, 1)
    print("Patch 1: content shader fields added")
else:
    print("Patch 1: fields already present")

# Patch 2: Add content shader compilation at end of new()
old_bcb = 'let bcb = make_dynamic_cbuffer(device, (BORDER_CB_FLOATS * 4) as u32)?;'
new_bcb = '''let bcb = make_dynamic_cbuffer(device, (BORDER_CB_FLOATS * 4) as u32)?;

        // ---- content overlay pipeline ----
        let cvs_blob = compile(crate::gfx::shaders::CONTENT_VS, "vs_main", "vs_5_0")?;
        let cps_blob = compile(crate::gfx::shaders::CONTENT_PS, "ps_main", "ps_5_0")?;
        let cvs = unsafe {
            let mut o = None;
            device
                .CreateVertexShader(bytes(&cvs_blob), None, Some(&mut o))
                .map_err(|e| e.to_string())?;
            o.ok_or("CreateVertexShader(content)")?
        };
        let cps = unsafe {
            let mut o = None;
            device
                .CreatePixelShader(bytes(&cps_blob), None, Some(&mut o))
                .map_err(|e| e.to_string())?;
            o.ok_or("CreatePixelShader(content)")?
        };
        let ccb = make_dynamic_cbuffer(device, (CONTENT_CB_FLOATS * 4) as u32)?;
        let csampler = make_linear_sampler(device)?;'''

assert old_bcb in content, "old_bcb not found"
content = content.replace(old_bcb, new_bcb, 1)
print("Patch 2: content shader compilation added")

# Patch 3: Add CONTENT_CB_FLOATS constant after BORDER_CB_FLOATS
old_const = "const BORDER_CB_FLOATS: usize = 7 * 4;"
new_const = "const BORDER_CB_FLOATS: usize = 7 * 4;\n/// ContentCb float count (c0..c2).\nconst CONTENT_CB_FLOATS: usize = 3 * 4;"
assert old_const in content, "old_const not found"
content = content.replace(old_const, new_const, 1)
print("Patch 3: CONTENT_CB_FLOATS constant added")

# Patch 4: Add cvs/cps/ccb/csampler to the Ok(Self{...}) construction
old_self = "            bcb,\n            lut_srv,"
new_self = "            bcb,\n            cvs,\n            cps,\n            ccb,\n            csampler,\n            lut_srv,"
assert old_self in content, "old_self not found"
content = content.replace(old_self, new_self, 1)
print("Patch 4: content fields in Self constructor added")

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("Written")
