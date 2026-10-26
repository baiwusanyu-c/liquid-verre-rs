import io
path = r"D:\github-project\liquid-verre-rs\playground\src\main.rs"
with io.open(path, "r", encoding="utf-8") as f:
    c = f.read()
c = c.replace("fn log_content_rect(mode: &str, tag: &str, s: &crate::api::WindowSnapshot) {",
              "fn log_content_rect(mode: &str, tag: &str, s: &liquid_verre::api::WindowSnapshot) {", 1)
old = """    for y in (y0..y1.min(h)) as usize {
        for x in (x0..x1.min(w)) as usize {
            let i = (y * w as usize + x) * 4;
            buf[i..i + 4].copy_from_slice(&VC_DIRTY_COLOR);
        }
    }"""
new = """    for y in (y0..y1.min(h)) as usize {
        for x in (x0..x1.min(w)) as usize {
            let i = (y * w as usize + x) * 4;
            buf[i..i + 4].copy_from_slice(&VC_DIRTY_COLOR);
        }
    }"""
c = c.replace("""    for y in (y0..y1.min(h)) as usize {
        for x in (x0..x1.min(w)) as usize {""",
"""    for y in (y0..y1.min(h)) {
        for x in (x0..x1.min(w)) {
            let (y, x) = (y as usize, x as usize);""", 1)
with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("fixed")
