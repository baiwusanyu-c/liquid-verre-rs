import io, os
p = r"D:\github-project\liquid-verre-rs\playground\src\main.rs"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()

# checker/solid immediate pushes: emit ready right after
old = '''        "checker" => {
            lower.update_content(w0, h0, &make_checker(w0, h0), None);
        }
        "solid" => {
            lower.update_content(w0, h0, &make_solid(w0, h0, [64, 128, 255, 255]), None);
        }'''
new = '''        "checker" => {
            lower.update_content(w0, h0, &make_checker(w0, h0), None);
            ready_after_frames(mode);
        }
        "solid" => {
            lower.update_content(w0, h0, &make_solid(w0, h0, [64, 128, 255, 255]), None);
            ready_after_frames(mode);
        }'''
assert old in c
c = c.replace(old, new, 1)

# dirty / dirty_none thread tails
old = """                w.update_content(w0, h0, &buf, dirty);
                log::info!(
                    target: "verify",
                    "dirty_push rect=[{},{},{},{}] color=({},{},{},{}) full_upload={}",
                    VC_DIRTY_RECT[0],
                    VC_DIRTY_RECT[1],
                    VC_DIRTY_RECT[2],
                    VC_DIRTY_RECT[3],
                    VC_DIRTY_COLOR[0],
                    VC_DIRTY_COLOR[1],
                    VC_DIRTY_COLOR[2],
                    VC_DIRTY_COLOR[3],
                    !use_dirty
                );"""
new = """                w.update_content(w0, h0, &buf, dirty);
                log::info!(
                    target: "verify",
                    "dirty_push rect=[{},{},{},{}] color=({},{},{},{}) full_upload={}",
                    VC_DIRTY_RECT[0],
                    VC_DIRTY_RECT[1],
                    VC_DIRTY_RECT[2],
                    VC_DIRTY_RECT[3],
                    VC_DIRTY_COLOR[0],
                    VC_DIRTY_COLOR[1],
                    VC_DIRTY_COLOR[2],
                    VC_DIRTY_COLOR[3],
                    !use_dirty
                );
                ready_after_frames(mode_owned.as_str());"""
assert old in c
c = c.replace(old, new, 1)
old = "            let w = lower.clone();\n            let use_dirty = mode == \"dirty\";"
new = "            let w = lower.clone();\n            let mode_owned = mode.to_string();\n            let use_dirty = mode == \"dirty\";"
assert old in c
c = c.replace(old, new, 1)

# clear_seq tail
old = """                w.clear_content();
                log::info!(target: "verify", "clear_push done");"""
new = """                w.clear_content();
                log::info!(target: "verify", "clear_push done");
                ready_after_frames("clear_seq");"""
assert old in c
c = c.replace(old, new, 1)

# stack: after push
old = """            log_content_rect(mode, "upper", &u);
            if mode == "stack" {
                lower.update_content(w0, h0, &make_checker(w0, h0), None);
            }
        }"""
new = """            log_content_rect(mode, "upper", &u);
            if mode == "stack" {
                lower.update_content(w0, h0, &make_checker(w0, h0), None);
            }
            ready_after_frames(mode);
        }"""
assert old in c
c = c.replace(old, new, 1)

# resize thread tail
old = """                log::info!(target: "verify", "resize_push new_tex={w1}x{h1} old_push={old_push} cleared={clear_mid}");"""
new = """                log::info!(target: "verify", "resize_push new_tex={w1}x{h1} old_push={old_push} cleared={clear_mid}");
                ready_after_frames(&label);"""
assert old in c
c = c.replace(old, new, 1)

# helper fn
anchor = "/// dirty 场景用的 checker：在 VC_DIRTY_RECT 内涂 VC_DIRTY_COLOR。"
helper = """/// 推流完成后，再让渲染线程跑几帧（UpdateContent/ClearContent 各消费一帧），
/// 然后打 `steady_ready` —— verify_content.py 以此行为准触发稳态帧截图，
/// 避免「shot 帧早于内容入队」的竞态（2026-10-26 bring-up 实测：静态桌面下
/// 首帧之后渲染门控会跳过，shot 必须挂在内容更新触发的那一帧上）。
fn ready_after_frames(mode: &str) {
    let m = mode.to_string();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        log::info!(target: "verify", "steady_ready mode={m}");
    });
}

"""
assert anchor in c
c = c.replace(anchor, helper + anchor, 1)

with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("playground steady_ready signals added")
