import io, os
p = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\glass_renderer.rs"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = """                self.cvs as *const _ as usize,
                self.cps as *const _ as usize,"""
new = """                &self.cvs as *const _ as usize,
                &self.cps as *const _ as usize,"""
assert old in c
c = c.replace(old, new, 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("ptr diag fixed")
