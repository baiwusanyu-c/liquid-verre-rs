import io, os
R = r"D:\github-project\liquid-verre-rs"

# 1) verify_content.py: read_bmp returns RGB
p = os.path.join(R, r"tools\verify_content.py")
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = """        for x in range(w):
            o = x * 4
            px.append((row[o], row[o + 1], row[o + 2], row[o + 3]))
    return w, h, px"""
new = """        for x in range(w):
            o = x * 4
            # BMP 像素数组是 BGRA；本脚本对外统一暴露 RGB 序，
            # 与 playground 图案常量 (DARK=30/LIGHT=220/DIRTY_COLOR 红) 同序。
            px.append((row[o + 2], row[o + 1], row[o], row[o + 3]))
    return w, h, px"""
assert old in c
c = c.replace(old, new, 1)
# DIRTY_COLOR constant is BGRA in playground; script compares RGB tuples now.
old2 = "DIRTY_RECT = (100, 100, 356, 356)   # playground VC_DIRTY_RECT"
new2 = "DIRTY_RECT = (100, 100, 356, 356)   # playground VC_DIRTY_RECT"
assert old2 in c
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("read_bmp -> RGB")

# 2) glass_renderer: remove DIAG5 + readback + READY_SEEN
p = os.path.join(R, r"crates\liquid-verre\src\gfx\glass_renderer.rs")
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
i0 = c.find("/// EXPERIMENT: copy 1 px of the bound RT")
i1 = c.find("pub(crate) static READY_SEEN")
assert i0 > 0 and i1 > i0
i2 = c.find("\n\n", c.find("}", i0))
# remove helper block: from i0 to the line "pub(crate) static READY_SEEN ... ;" inclusive
j = c.find("\n", c.find("READY_SEEN: std::sync::atomic::AtomicBool::new(false);"))
c = c[:i0].rstrip("\n") + "\n" + c[j+1:]
old = """            ctx.Draw(6, 0);
            static ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
            if READY_SEEN.load(std::sync::atomic::Ordering::Relaxed)
                && ONCE.swap(false, std::sync::atomic::Ordering::Relaxed)
            {
                log::info!(
                    target: "verify",
                    "DIAG6 post-ready readback: {}",
                    readback_probe(ctx, device_for_readback, rx0 + 200, ry0 + 200)
                );
            }"""
new = "            ctx.Draw(6, 0);"
assert old in c, "DIAG6 block"
c = c.replace(old, new, 1)
old = "    fn draw_content_for_block(\n        &self,\n        ctx: &ID3D11DeviceContext,\n        device_for_readback: &ID3D11Device,\n        block: &DrawBlock,"
new = "    fn draw_content_for_block(\n        &self,\n        ctx: &ID3D11DeviceContext,\n        block: &DrawBlock,"
assert old in c
c = c.replace(old, new, 1)
old = """                    self.draw_content_for_block(
                        ctx,
                        device,
                        block,"""
new = """                    self.draw_content_for_block(
                        ctx,
                        block,"""
assert old in c
c = c.replace(old, new, 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("glass_renderer de-experimented")

# 3) app.rs: remove READY_SEEN arming
p = os.path.join(R, r"crates\liquid-verre\src\runtime\app.rs")
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = """    // EXPERIMENT: arm the one-shot readback once the app has been up > 2.5 s.
    if !crate::gfx::glass_renderer::READY_SEEN.load(std::sync::atomic::Ordering::Relaxed)
        && app.t0.elapsed().as_millis() > 2500
    {
        crate::gfx::glass_renderer::READY_SEEN.store(true, std::sync::atomic::Ordering::Relaxed);
    }
"""
assert old in c
c = c.replace(old, "", 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("app.rs de-experimented")
