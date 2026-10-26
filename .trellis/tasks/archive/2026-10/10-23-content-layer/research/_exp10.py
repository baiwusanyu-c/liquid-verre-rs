import io, os
p = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\glass_renderer.rs"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = """    unsafe {
        let mut rt = None;
        let mut ds = None;
        ctx.OMGetRenderTargets(0, Some(&mut rt), Some(&mut ds));
        let Some(rt) = rt else {
            return "NO_RT_BOUND".to_string();
        };
        let mut tex = None;
        if rt.QueryInterface(&mut tex).is_err() {
            return "RT_NOT_TEXTURE".to_string();
        }
        let tex: windows::Win32::Graphics::Direct3D11::ID3D11Texture2D = tex;"""
new = """    use windows::core::Interface;
    use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
    unsafe {
        let mut rts: [Option<ID3D11RenderTargetView>; 1] = [None];
        let mut ds: Option<ID3D11DepthStencilView> = None;
        ctx.OMGetRenderTargets(1, Some(&mut rts), Some(&mut ds));
        let Some(rt) = rts[0].clone() else {
            return "NO_RT_BOUND".to_string();
        };
        let tex: ID3D11Texture2D = match rt.QueryInterface() {
            Ok(t) => t,
            Err(_) => return "RT_NOT_TEXTURE".to_string(),
        };"""
assert old in c
c = c.replace(old, new, 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("readback fixed")
