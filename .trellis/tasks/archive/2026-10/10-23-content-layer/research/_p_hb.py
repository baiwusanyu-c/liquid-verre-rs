import io, os
p = r"D:\github-project\liquid-verre-rs\playground\src\main.rs"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()

# ready_after_frames: heartbeat variant with window handle
old = """fn ready_after_frames(mode: &str) {
    let m = mode.to_string();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        log::info!(target: "verify", "steady_ready mode={m}");
    });
}"""
new = """fn ready_after_frames(mode: &str, w: GlassWindow) {
    let m = mode.to_string();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        log::info!(target: "verify", "steady_ready mode={m}");
        // 心跳：每 150ms 发一条 no-op SetVisible 触发渲染帧（push() 自带
        // request_render），保证稳态帧 deadline 到点时有帧可拍、且内容已入 store。
        // 没有心跳时静态桌面下渲染门控会跳过 shot 之后的所有帧（2026-10-26）。
        for _ in 0..30 {
            w.set_visible(true);
            std::thread::sleep(Duration::from_millis(150));
        }
    });
}"""
assert old in c
c = c.replace(old, new, 1)

# call sites need the handle
c = c.replace("""            lower.update_content(w0, h0, &make_checker(w0, h0), None);
            ready_after_frames(mode);""",
"""            lower.update_content(w0, h0, &make_checker(w0, h0), None);
            ready_after_frames(mode, lower.clone());""", 1)
c = c.replace("""            lower.update_content(w0, h0, &make_solid(w0, h0, [64, 128, 255, 255]), None);
            ready_after_frames(mode);""",
"""            lower.update_content(w0, h0, &make_solid(w0, h0, [64, 128, 255, 255]), None);
            ready_after_frames(mode, lower.clone());""", 1)
c = c.replace('                ready_after_frames(mode_owned.as_str());',
              '                ready_after_frames(&mode_owned, w.clone());', 1)
c = c.replace('                ready_after_frames("clear_seq");',
              '                ready_after_frames("clear_seq", w.clone());', 1)
c = c.replace("""            ready_after_frames(mode);
        }""",
"""            ready_after_frames(mode, lower.clone());
        }""", 1)
c = c.replace('                ready_after_frames(&label);',
              '                ready_after_frames(&label, w.clone());', 1)

with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("heartbeat ready signal done")
