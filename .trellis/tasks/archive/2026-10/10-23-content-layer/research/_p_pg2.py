import io
path = r"D:\github-project\liquid-verre-rs\playground\src\main.rs"
with io.open(path, "r", encoding="utf-8") as f:
    c = f.read()

# Clean up the sloppy origin placeholder in log_content_rect
old = """    let sc = s.scale.max(0.05);
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
    let y1 = ((s.rect[1] + s.rect[3]) * sc - oy as f32).round();"""
new = """    // 取证 hermetic：GLASS_MONITORS=1 且主屏原点恒 (0,0)，画布局部 = 虚拟屏物理像素。
    let sc = s.scale.max(0.05);
    let x0 = (s.rect[0] * sc).round();
    let y0 = (s.rect[1] * sc).round();
    let x1 = ((s.rect[0] + s.rect[2]) * sc).round();
    let y1 = ((s.rect[1] + s.rect[3]) * sc).round();"""
assert old in c, "placeholder not found"
c = c.replace(old, new, 1)

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("cleaned")
