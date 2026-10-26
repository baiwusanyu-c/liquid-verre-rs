import io, os
R = r"D:\github-project\liquid-verre-rs"
p = os.path.join(R, r"crates\liquid-verre\src\gfx\glass_renderer.rs")
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
i_ready = c.find("pub(crate) static READY_SEEN")
i_help = c.find("/// EXPERIMENT: copy 1 px")
print("ready@", i_ready, "helper@", i_help)
# block to delete: from min to end of helper function (helper ends before the next top-level item)
start = min(x for x in (i_ready, i_help) if x >= 0)
# find end of helper: the line "}" at column 0 after helper start
end = c.find("\n}\n", c.find("fn readback_probe", start)) + 3
print("delete block:", repr(c[start:start+60]), "...", repr(c[end-40:end+60]))
c = c[:start].rstrip("\n") + "\n\n" + c[end:].lstrip("\n")
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
print("glass_renderer cleaned")

pa = os.path.join(R, r"crates\liquid-verre\src\runtime\app.rs")
with io.open(pa, "r", encoding="utf-8") as f:
    a = f.read()
old = """    // EXPERIMENT: arm the one-shot readback once the app has been up > 2.5 s.
    if !crate::gfx::glass_renderer::READY_SEEN.load(std::sync::atomic::Ordering::Relaxed)
        && app.t0.elapsed().as_millis() > 2500
    {
        crate::gfx::glass_renderer::READY_SEEN.store(true, std::sync::atomic::Ordering::Relaxed);
    }
"""
assert old in a, "app arm block"
a = a.replace(old, "", 1)
with io.open(pa, "w", encoding="utf-8", newline="\n") as f:
    f.write(a)
print("app.rs cleaned")
