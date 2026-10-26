import io
path = r"D:\github-project\liquid-verre-rs\playground\src\main.rs"
with io.open(path, "r", encoding="utf-8") as f:
    c = f.read()
c = c.replace("for y in (y0..y1.min(h)) {", "for y in y0..y1.min(h) {", 1)
c = c.replace("for x in (x0..x1.min(w)) {", "for x in x0..x1.min(w) {", 1)
c = c.replace("            let upper = engine.create_window(WindowConfig {", "            let _upper = engine.create_window(WindowConfig {", 1)
with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("warnings fixed")
