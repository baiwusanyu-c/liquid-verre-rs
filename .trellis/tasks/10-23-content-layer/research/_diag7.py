import io
p = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\glass_renderer.rs"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = "if store.get(block.id).is_some() && DIAG3.swap(true, std::sync::atomic::Ordering::Relaxed) == false {"
new = "if store.get(block.id).is_some() && DIAG3.swap(false, std::sync::atomic::Ordering::Relaxed) {"
assert old in c
c = c.replace(old, new, 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("DIAG3 polarity fixed")
