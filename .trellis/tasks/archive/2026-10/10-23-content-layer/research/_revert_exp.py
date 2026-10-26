import io, os
R = r"D:\github-project\liquid-verre-rs"

# restore real shaders
vs = os.path.join(R, r"crates\liquid-verre\shaders\content_vs.hlsl")
ps = os.path.join(R, r"crates\liquid-verre\shaders\content_ps.hlsl")
for f in (vs, ps):
    keep = f + ".keep"
    if os.path.exists(keep):
        io.open(f, "w", encoding="utf-8", newline="\n").write(io.open(keep, encoding="utf-8").read())
        os.unlink(keep)
ps_orig = ps + ".orig"
if os.path.exists(ps_orig):
    os.unlink(ps_orig)

# glass_renderer: restore scissor, drop diag logs
gp = os.path.join(R, r"crates\liquid-verre\src\gfx\glass_renderer.rs")
with io.open(gp, "r", encoding="utf-8") as f:
    c = f.read()
old = "            ctx.RSSetScissorRects(None); // EXPERIMENT: no scissor\n            ctx.VSSetShader(&self.cvs, None);"
new = """            ctx.RSSetScissorRects(Some(&[WinRect {
                left: sx,
                top: sy,
                right: cx1,
                bottom: cy1,
            }]));
            ctx.VSSetShader(&self.cvs, None);"""
assert old in c
c = c.replace(old, new, 1)
old = """            ctx.Draw(6, 0);
            log::info!(
                target: "verify",
                "DIAG Draw(6) issued id={} vs={} ps={} scissor=({},{},{},{})",
                block.id,
                &self.cvs as *const _ as usize,
                &self.cps as *const _ as usize,
                sx,
                sy,
                cx1,
                cy1
            );
"""
new = "            ctx.Draw(6, 0);\n"
assert old in c
c = c.replace(old, new, 1)
old = "    /// TEMP DIAG: one-shot flag for draw_content log\n    diag_logged: std::cell::Cell<bool>,\n"
assert old in c
c = c.replace(old, "", 1)
old = "            drawn: std::cell::Cell::new(0),\n            diag_logged: std::cell::Cell::new(false),\n        })"
new = "            drawn: std::cell::Cell::new(0),\n        })"
assert old in c
c = c.replace(old, new, 1)
old = "        if let Err(e) = update_cbuffer(ctx, &self.ccb, &cb) {\n            log::info!(target: \"verify\", \"DIAG ccb update failed: {e}\");\n            return;\n        }"
new = "        if update_cbuffer(ctx, &self.ccb, &cb).is_err() {\n            return;\n        }"
assert old in c
c = c.replace(old, new, 1)
with io.open(gp, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("glass_renderer diag/scissor restored")

# commands.rs: drop cmd diag log
cp = os.path.join(R, r"crates\liquid-verre\src\runtime\commands.rs")
with io.open(cp, "r", encoding="utf-8") as f:
    c = f.read()
old = """            log::info!(
                target: "verify",
                "DIAG cmd UpdateContent id={id} {w_phys}x{h_phys} len={} layers={}",
                buffer.len(),
                app.content_store.diag_len()
            );
"""
assert old in c
c = c.replace(old, "", 1)
with io.open(cp, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("commands diag restored")

# content_layer: drop diag_len
lp = os.path.join(R, r"crates\liquid-verre\src\gfx\content_layer.rs")
with io.open(lp, "r", encoding="utf-8") as f:
    c = f.read()
old = """    /// TEMP DIAG (remove after content-layer bring-up): store size.
    pub fn diag_len(&self) -> usize {
        self.layers.len()
    }

"""
assert old in c
c = c.replace(old, "", 1)
with io.open(lp, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("content_layer diag restored")

# playground: move demo dispatch BEFORE panel::start
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
new = """    let panel = match panel::start"""
assert old in c
c = c.replace(old, new, 1)
old2 = """    // 输入路由 demo 挂载"""
if old2 not in c:
    # find the sink demo anchor
    i = c.find("GLASS_INPUT_SINK_DEMO")
    i2 = c.rfind("//", 0, i)
    print("sink demo anchor:", repr(c[i2:i2+80]))
    old2 = c[i2:i2+len("// GlassInputSink")]
anchor = "    let panel = match panel::start"
idx = c.find(anchor)
assert idx > 0
demo_block = """    // 内容 demo 必须**先于** panel 启动：panel::bind 会占用渲染线程命令队列若干帧，
    // 且 verify_content 的稳态帧窗口（2.5s）要全部留给推流/重绘，不能被面板启动抢占。
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
