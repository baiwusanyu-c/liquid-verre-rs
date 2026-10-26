import io
p = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\glass_renderer.rs"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = "    /// TEMP DIAG: one-shot flag for draw_content log\n    diag_logged: std::cell::Cell<bool>,\n"
assert old in c
c = c.replace(old, "", 1)
old2 = "            drawn: std::cell::Cell::new(0),\n            diag_logged: std::cell::Cell::new(false),\n        })"
if old2 in c:
    c = c.replace(old2, "            drawn: std::cell::Cell::new(0),\n        })", 1)
    print("init also removed")
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("diag_logged removed")
