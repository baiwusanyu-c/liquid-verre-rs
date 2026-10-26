import os, time
R = r"D:\github-project\liquid-verre-rs"
for p in [os.path.join(R, r"crates\liquid-verre\src\gfx\glass_renderer.rs"),
          os.path.join(R, r"crates\liquid-verre\src\gfx\compositor.rs")]:
    os.utime(p, None)
    print("touched", p)
