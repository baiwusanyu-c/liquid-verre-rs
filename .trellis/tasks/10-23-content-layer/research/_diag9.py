import io
p = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\glass_renderer.rs"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = """            ctx.VSSetShader(&self.cvs, None);
            ctx.PSSetShader(&self.cps, None);
            ctx.Draw(6, 0);
            // 切回玻璃管线。"""
new = """            ctx.VSSetShader(&self.cvs, None);
            ctx.PSSetShader(&self.cps, None);
            ctx.Draw(6, 0);
            static ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
            if ONCE.swap(false, std::sync::atomic::Ordering::Relaxed) {
                log::info!(
                    target: "verify",
                    "DIAG5 content Draw executed id={} scissor=({},{},{},{})",
                    block.id,
                    sx,
                    sy,
                    cx1,
                    cy1
                );
            }
            // 切回玻璃管线。"""
assert old in c
c = c.replace(old, new, 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("DIAG5 added")
