import io
path = r"D:\github-project\liquid-verre-rs\playground\src\main.rs"
with io.open(path, "r", encoding="utf-8") as f:
    c = f.read()

old = """            let w = lower.clone();
            let old_push = mode != "resize_direct";
            let clear_mid = mode == "resize_seq";
            std::thread::spawn(move || {
                if old_push {
                    std::thread::sleep(Duration::from_millis(300));
                    w.update_content(w0, h0, &make_checker(w0, h0), None);
                }
                std::thread::sleep(Duration::from_millis(500));
                w.set_size(nw, nh);
                if clear_mid {
                    std::thread::sleep(Duration::from_millis(300));
                    w.clear_content();
                }
                std::thread::sleep(Duration::from_millis(400));
                w.update_content(w1, h1, &make_checker(w1, h1), None);
                log::info!(target: "verify", "resize_push new_tex={w1}x{h1} old_push={old_push} cleared={clear_mid}");
            });"""
new = """            let w = lower.clone();
            let old_push = mode != "resize_direct";
            let clear_mid = mode == "resize_seq";
            let label = mode.to_string();
            std::thread::spawn(move || {
                if old_push {
                    std::thread::sleep(Duration::from_millis(300));
                    w.update_content(w0, h0, &make_checker(w0, h0), None);
                }
                std::thread::sleep(Duration::from_millis(500));
                w.set_size(nw, nh);
                // resize 后的新块几何 = 取证断言 5 的采样锚点，等快照发布再打日志。
                std::thread::sleep(Duration::from_millis(300));
                if let Some(s2) = w.snapshot() {
                    log_content_rect(&label, "lower_after", &s2);
                }
                if clear_mid {
                    std::thread::sleep(Duration::from_millis(100));
                    w.clear_content();
                }
                std::thread::sleep(Duration::from_millis(400));
                w.update_content(w1, h1, &make_checker(w1, h1), None);
                log::info!(target: "verify", "resize_push new_tex={w1}x{h1} old_push={old_push} cleared={clear_mid}");
            });"""
assert old in c, "resize branch not found"
c = c.replace(old, new, 1)
with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("resize lower_after log added")
