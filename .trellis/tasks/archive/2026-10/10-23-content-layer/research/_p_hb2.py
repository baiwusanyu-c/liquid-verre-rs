import io
p = r"D:\github-project\liquid-verre-rs\playground\src\main.rs"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
c = c.replace("fn ready_after_frames(mode: &str, w: GlassWindow) {",
              "fn ready_after_frames(mode: &str, w: liquid_verre::api::GlassWindow) {", 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("type path fixed")
