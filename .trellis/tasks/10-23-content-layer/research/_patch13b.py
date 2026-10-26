import io

path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\glass_renderer.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()

# Patch 1: Add content shader fields
old_f = "    bcb: ID3D11Buffer,\n    pub env_ema: [f32; 3],"
new_f = "    bcb: ID3D11Buffer,\n    cvs: ID3D11VertexShader,\n    cps: ID3D11PixelShader,\n    ccb: ID3D11Buffer,\n    csampler: ID3D11SamplerState,\n    pub env_ema: [f32; 3],"
assert old_f in content, "Patch1: fields not matched"
content = content.replace(old_f, new_f, 1)
print("Patch 1: content fields added")

# Patch 2: Add CONTENT_CB_FLOATS
old_c = "const BORDER_CB_FLOATS: usize = 7 * 4;"
new_c = "const BORDER_CB_FLOATS: usize = 7 * 4;\nconst CONTENT_CB_FLOATS: usize = 3 * 4;"
assert old_c in content, "Patch2: const not found"
content = content.replace(old_c, new_c, 1)
print("Patch 2: CONTENT_CB_FLOATS added")

# Patch 3: Add content shader compilation (after border cbuffer)
old_b = "let bcb = make_dynamic_cbuffer(device, (BORDER_CB_FLOATS * 4) as u32)?;"
new_b = '''let bcb = make_dynamic_cbuffer(device, (BORDER_CB_FLOATS * 4) as u32)?;

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
assert old_b in content, "Patch3: bcb not found"
content = content.replace(old_b, new_b, 1)
print("Patch 3: content compilation added")

# Patch 4: Add fields to Ok(Self{...})
old_s = "            bcb,\n            lut_srv,"
new_s = "            bcb,\n            cvs,\n            cps,\n            ccb,\n            csampler,\n            lut_srv,"
assert old_s in content, "Patch4: Self not found"
content = content.replace(old_s, new_s, 1)
print("Patch 4: Self fields added")

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("All patches written")
