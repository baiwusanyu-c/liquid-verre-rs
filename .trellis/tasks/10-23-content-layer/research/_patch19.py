import io

path = r"D:\github-project\liquid-verre-rs\playground\src\main.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()

# Add content demo after the input_sink_demo block (before "let panel = match panel::start")
old = r'''            log::warn!("[playground] GLASS_INPUT_SINK_DEMO: 默认块 snapshot 不可用，不挂载");
        }
    }

    let panel = match panel::start(Arc::clone(&engine), opts.port, opts.no_browser) {'''

new = r'''            log::warn!("[playground] GLASS_INPUT_SINK_DEMO: 默认块 snapshot 不可用，不挂载");
        }
    }

    // Content demo: GLASS_CONTENT_DEMO=checker|solid  pushes deterministic
    // BGRA8 content to the single default block (hermetic: GLASS_DEFAULT_BLOCK=1).
    if let Some(demo) = std::env::var_os("GLASS_CONTENT_DEMO") {
        let mode = demo.to_string_lossy().to_lowercase();
        if mode != "off" && mode != "0" && mode != "" {
            wait_for_monitors(&engine);
            let wins = engine.windows();
            if wins.len() != 1 {
                log::warn!(
                    "[playground] GLASS_CONTENT_DEMO={} requires single block (wins={}), skipping",
                    mode, wins.len()
                );
            } else if let Some(s) = wins[0].snapshot() {
                let w_phys = (s.rect[2] * s.scale).ceil() as u32;
                let h_phys = (s.rect[3] * s.scale).ceil() as u32;
                let buf = match mode.as_str() {
                    "checker" => make_checker(w_phys, h_phys),
                    _ => make_solid(w_phys, h_phys, [64, 128, 255, 255]),
                };
                wins[0].update_content(w_phys, h_phys, &buf, None);
                log::info!(
                    "[playground] GLASS_CONTENT_DEMO={}: pushed {}x{} content to block {}",
                    mode, w_phys, h_phys, s.id
                );
            } else {
                log::warn!("[playground] GLASS_CONTENT_DEMO: snapshot unavailable, skipping");
            }
        }
    }

    let panel = match panel::start(Arc::clone(&engine), opts.port, opts.no_browser) {'''

assert old in content, "old block not found"
content = content.replace(old, new, 1)
print("Content demo block added")

# Add checker/solid helper functions before fn main() or at end of file
# Find position after SinkDemo
old2 = "fn layout_from_env() -> bool {"
new2 = """/// Generate a deterministic 16x16 checkerboard pattern (BGRA8, pre-multiplied).
fn make_checker(w: u32, h: u32) -> Vec<u8> {
    let size = (w as usize) * (h as usize) * 4;
    let mut buf = vec![0u8; size];
    let tile = 16usize;
    for y in 0..(h as usize) {
        for x in 0..(w as usize) {
            let idx = (y * (w as usize) + x) * 4;
            let black = ((x / tile) + (y / tile)) % 2 == 0;
            if black {
                buf[idx] = 30;
                buf[idx + 1] = 30;
                buf[idx + 2] = 30;
                buf[idx + 3] = 255;
            } else {
                buf[idx] = 220;
                buf[idx + 1] = 220;
                buf[idx + 2] = 220;
                buf[idx + 3] = 255;
            }
        }
    }
    buf
}

/// Generate a solid color fill (BGRA8, pre-multiplied).
fn make_solid(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let size = (w as usize) * (h as usize) * 4;
    let mut buf = vec![0u8; size];
    // Pre-multiply: for opaque alpha=255, color stays as-is.
    let a = rgba[3] as f32 / 255.0;
    let b = (rgba[0] as f32 * a) as u8;
    let g = (rgba[1] as f32 * a) as u8;
    let r = (rgba[2] as f32 * a) as u8;
    for i in (0..size).step_by(4) {
        buf[i] = b;
        buf[i + 1] = g;
        buf[i + 2] = r;
        buf[i + 3] = rgba[3];
    }
    buf
}

fn layout_from_env() -> bool {"""
assert old2 in content, "layout_from_env not found"
content = content.replace(old2, new2, 1)
print("Checker/solid helpers added")

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("Written")
