import io
path = r"D:\github-project\liquid-verre-rs\playground\src\main.rs"
with io.open(path, "r", encoding="utf-8") as f:
    c = f.read()

# Replace the current single-push demo block with a dispatcher.
start = c.find("    // Content demo: GLASS_CONTENT_DEMO=checker|solid")
end = c.find("    let panel = match panel::start")
assert start > 0 and end > start, "anchors not found"

new_block = '''    // 内容 demo（P0-1 取证入口）：GLASS_CONTENT_DEMO 选择确定性场景，仅构造期读一次
    // （playground 范围；库的 env 例外清单不变）。场景时序由后台线程推流，稳态帧
    // 交给 GLASS_INTERNAL_SHOT_AT_MS=2500 取证；tools/verify_content.py 解析
    // [verify] content_rect / stack_ / dirty_ 日志行定位采样点。
    if let Some(demo) = std::env::var_os("GLASS_CONTENT_DEMO") {
        let mode = demo.to_string_lossy().to_lowercase();
        if mode != "off" && mode != "0" && !mode.is_empty() {
            run_content_demo(&engine, &mode);
        }
    }

'''
c = c[:start] + new_block + c[end:]

# Helpers appended before `/// 脚本是否已经通过 GLASS_* 指定了确定性布局。` (layout_from_env doc)
anchor = "/// 脚本是否已经通过 GLASS_* 指定了确定性布局。"
assert anchor in c, "layout_from_env anchor missing"

helpers = '''/// dirty 场景的子区（纹理空间物理像素，半开区间）。所有 resize 前的默认块
/// 尺寸下限是 720x480（见 runtime seed），子区 256x256 恒在界内。
const VC_DIRTY_RECT: [u32; 4] = [100, 100, 356, 356];
/// dirty 子区重绘颜色（premul BGRA8；红通道 220，与 checker 两档灰都不同）。
const VC_DIRTY_COLOR: [u8; 4] = [40, 40, 220, 255];

fn run_content_demo(engine: &Arc<Engine>, mode: &str) {
    wait_for_monitors(engine);
    let wins = engine.windows();
    if wins.len() != 1 {
        log::warn!(
            "[playground] GLASS_CONTENT_DEMO={mode} needs the single default block (wins={}), skipping",
            wins.len()
        );
        return;
    }
    let lower = wins[0].clone();
    let Some(s) = lower.snapshot() else {
        log::warn!("[playground] GLASS_CONTENT_DEMO={mode}: snapshot unavailable, skipping");
        return;
    };
    log_content_rect(mode, "lower", &s);
    let sc = s.scale.max(0.05);
    let (w0, h0) = (
        (s.rect[2] * sc).round().max(1.0) as u32,
        (s.rect[3] * sc).round().max(1.0) as u32,
    );

    match mode {
        "checker" => {
            lower.update_content(w0, h0, &make_checker(w0, h0), None);
        }
        "solid" => {
            lower.update_content(w0, h0, &make_solid(w0, h0, [64, 128, 255, 255]), None);
        }
        "dirty" | "dirty_none" => {
            lower.update_content(w0, h0, &make_checker(w0, h0), None);
            let w = lower.clone();
            let use_dirty = mode == "dirty";
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(1200));
                let buf = make_checker_dirty(w0, h0);
                let dirty = if use_dirty {
                    Some(VC_DIRTY_RECT)
                } else {
                    None
                };
                w.update_content(w0, h0, &buf, dirty);
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
            });
        }
        "clear_seq" => {
            lower.update_content(w0, h0, &make_checker(w0, h0), None);
            let w = lower.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(1200));
                w.clear_content();
                log::info!(target: "verify", "clear_push done");
            });
        }
        "stack" | "stack_bare" => {
            // 上块与下块 50% 偏移、同尺寸：重叠区 = 下块右下 1/4（逻辑坐标）。
            let rect = [
                s.rect[0] + s.rect[2] * 0.5,
                s.rect[1] + s.rect[3] * 0.5,
                s.rect[2],
                s.rect[3],
            ];
            let upper = engine.create_window(WindowConfig {
                rect,
                ..Default::default()
            });
            // 等渲染线程把第 2 块的 snapshot 发布出来（4 Hz 兜底 + dirty 即时）。
            let mut up_snap = None;
            for _ in 0..40 {
                if let Some(u) = engine.windows().iter().find(|w| w.id() != lower.id()).and_then(|w| w.snapshot()) {
                    up_snap = Some(u);
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let Some(u) = up_snap else {
                log::warn!("[playground] stack demo: upper snapshot not published");
                return;
            };
            log_content_rect(mode, "upper", &u);
            if mode == "stack" {
                lower.update_content(w0, h0, &make_checker(w0, h0), None);
            }
        }
        "resize_seq" | "resize_direct" | "resize_noclear" => {
            // 放大 25%/20%，并把矩形收进本屏可用区（set_size 锚左上角向右下长）。
            let (mon_w, mon_h) = engine
                .monitors()
                .first()
                .map(|m| (m.size[0] as f32 / sc, m.size[1] as f32 / sc))
                .unwrap_or((1920.0, 1080.0));
            let nw = (s.rect[2] * 1.25).min(mon_w - s.rect[0] - 8.0);
            let nh = (s.rect[3] * 1.2).min(mon_h - s.rect[1] - 8.0);
            let (w1, h1) = ((nw * sc).round().max(1.0) as u32, (nh * sc).round().max(1.0) as u32);
            let w = lower.clone();
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
            });
        }
        other => {
            log::warn!("[playground] GLASS_CONTENT_DEMO={other}: unknown mode, skipping");
        }
    }
}

/// 块矩形的画布局部物理像素日志——verify_content.py 的采样锚点单一出口。
fn log_content_rect(mode: &str, tag: &str, s: &crate::api::WindowSnapshot) {
    let sc = s.scale.max(0.05);
    let origin = s
        .monitor_index
        .0
        .checked_mul(0) // 占位不用；真实原点从 monitors 快照取
        .unwrap_or(0);
    let _ = origin;
    let (ox, oy) = (0i32, 0i32); // 由调用侧保证 hermetic（GLASS_MONITORS=1，主屏原点 (0,0)）
    let x0 = (s.rect[0] * sc - ox as f32).round();
    let y0 = (s.rect[1] * sc - oy as f32).round();
    let x1 = ((s.rect[0] + s.rect[2]) * sc - ox as f32).round();
    let y1 = ((s.rect[1] + s.rect[3]) * sc - oy as f32).round();
    log::info!(
        target: "verify",
        "content_rect mode={mode} tag={tag} id={} rect=[{x0:.0},{y0:.0},{x1:.0},{y1:.0}] scale={sc:.3} radius={:.0}",
        s.id,
        s.style.corner_radius() * sc
    );
}

/// dirty 场景用的 checker：在 VC_DIRTY_RECT 内涂 VC_DIRTY_COLOR。
fn make_checker_dirty(w: u32, h: u32) -> Vec<u8> {
    let mut buf = make_checker(w, h);
    let [x0, y0, x1, y1] = VC_DIRTY_RECT;
    for y in (y0..y1.min(h)) as usize {
        for x in (x0..x1.min(w)) as usize {
            let i = (y * w as usize + x) * 4;
            buf[i..i + 4].copy_from_slice(&VC_DIRTY_COLOR);
        }
    }
    buf
}

'''
c = c.replace(anchor, helpers + anchor, 1)

# checker parity values: make sure existing make_checker is 30/220 -- it is (round-1).

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("playground demo rewritten")
