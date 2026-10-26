import io
path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\runtime\commands.rs"
with io.open(path, "r", encoding="utf-8") as f:
    c = f.read()

old = """        Command::UpdateContent {
            id,
            w_phys,
            h_phys,
            buffer,
            dirty: _dirty,
        } => {
            // Content store update triggers a render request; the block's existing
            // dirty/influence-box mechanism is reused (C4: unified invalidation).
            app.content_store.update(
                &app.gfx.device,
                &app.gfx.ctx,
                id,
                w_phys,
                h_phys,
                &buffer,
                None,
            );
            // Mark the block's monitor region dirty so it gets redrawn.
            if let Some(b) = app.manager.get(id) {
                let mi = b.monitor;
                if let Some(m) = app.monitors.get_mut(mi) {
                    m.region_dirty = true;
                }
            }
            app.shared.request_render();
            true
        }
        Command::ClearContent { id } => {
            app.content_store.clear(id);
            false
        }"""

new = """        Command::UpdateContent {
            id,
            w_phys,
            h_phys,
            buffer,
            dirty,
        } => {
            // 失效走既有统一收口（C4）：返回 true => 主循环 need_render => 整帧重画
            // （Pass B 本来就画全部块，不变量 1）。不新增第二套脏区账本；
            // region 由块矩形决定，内容更新**不**改窗口区域，所以**不**置 region_dirty
            // （上一版误置导致每帧 SetWindowRgn 重建——已删）。
            app.content_store.update(
                &app.gfx.device,
                &app.gfx.ctx,
                id,
                w_phys,
                h_phys,
                &buffer,
                dirty,
            );
            true
        }
        Command::ClearContent { id } => {
            app.content_store.clear(id);
            // true 是行为正确性要求（2026-10-26 回炉发现）：清除**改变画面**，必须触发
            // 一帧重画。返回 false 时静止桌面上的 clear 要等下一个无关触发源才生效，
            // verify_content.py 断言 3 会直接抓到这个。
            true
        }"""
assert old in c, "old handlers not found"
c = c.replace(old, new, 1)

# DestroyAll: release all content textures
old_da = """            app.manager.clear();
            app.style_cache.clear();"""
new_da = """            app.manager.clear();
            app.content_store.clear_all();
            app.style_cache.clear();"""
assert old_da in c, "DestroyAll not found"
c = c.replace(old_da, new_da, 1)

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("commands.rs fixed")
