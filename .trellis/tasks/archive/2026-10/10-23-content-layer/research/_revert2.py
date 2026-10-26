import io, os
R = r"D:\github-project\liquid-verre-rs"

gp = os.path.join(R, r"crates\liquid-verre\src\gfx\glass_renderer.rs")
with io.open(gp, "r", encoding="utf-8") as f:
    c = f.read()
old = '        if let Err(e) = update_cbuffer(ctx, &self.ccb, &cb) {\n            log::info!(target: "verify", "DIAG ccb update failed: {e}");\n            return;\n        }'
new = '        if update_cbuffer(ctx, &self.ccb, &cb).is_err() {\n            return;\n        }'
assert old in c, "ccb diag"
c = c.replace(old, new, 1)
with io.open(gp, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("glass_renderer fully de-diagged")

cp = os.path.join(R, r"crates\liquid-verre\src\runtime\commands.rs")
with io.open(cp, "r", encoding="utf-8") as f:
    c = f.read()
old = '            log::info!(\n                target: "verify",\n                "DIAG cmd UpdateContent id={id} {w_phys}x{h_phys} len={} layers={}",\n                buffer.len(),\n                app.content_store.diag_len()\n            );\n'
assert old in c, "cmd diag"
c = c.replace(old, "", 1)
with io.open(cp, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("commands de-diagged")

lp = os.path.join(R, r"crates\liquid-verre\src\gfx\content_layer.rs")
with io.open(lp, "r", encoding="utf-8") as f:
    c = f.read()
old = "    /// TEMP DIAG (remove after content-layer bring-up): store size.\n    pub fn diag_len(&self) -> usize {\n        self.layers.len()\n    }\n\n"
assert old in c, "diag_len"
c = c.replace(old, "", 1)
with io.open(lp, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("content_layer de-diagged")

pp = os.path.join(R, r"playground\src\main.rs")
with io.open(pp, "r", encoding="utf-8") as f:
    c = f.read()
old = """    if let Some(demo) = std::env::var_os("GLASS_CONTENT_DEMO") {
        let mode = demo.to_string_lossy().to_lowercase();
        if mode != "off" && mode != "0" && !mode.is_empty() {
            run_content_demo(&engine, &mode);
        }
    }

    let panel = match panel::start"""
new = "    let panel = match panel::start"
assert old in c, "demo block at old position"
c = c.replace(old, new, 1)
anchor = "    let panel = match panel::start"
idx = c.find(anchor)
assert idx > 0
demo_block = """    // 内容 demo 的分发排在 panel 之前：panel::start 会占掉渲染线程头几帧，
    // verify_content 的稳态帧窗口（2.5s）必须全部留给推流 + 交错重绘。
    if let Some(demo) = std::env::var_os("GLASS_CONTENT_DEMO") {
        let mode = demo.to_string_lossy().to_lowercase();
        if mode != "off" && mode != "0" && !mode.is_empty() {
            run_content_demo(&engine, &mode);
        }
    }

"""
c = c[:idx] + demo_block + c[idx:]
with io.open(pp, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("playground: demo dispatch moved before panel")
