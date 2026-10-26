import io, shutil, os
R = r"D:\github-project\liquid-verre-rs"
research = os.path.join(R, ".trellis", "tasks", "10-23-content-layer", "research")
shutil.copy2(os.path.join(R, r"target\release\liquid-verre-playground.exe"),
             os.path.join(research, "vc_batch_probe.exe"))
# restore interleaved sources
shutil.copy2(os.path.join(research, "glass_renderer.interleaved.bak"),
             os.path.join(R, r"crates\liquid-verre\src\gfx\glass_renderer.rs"))
shutil.copy2(os.path.join(research, "compositor.interleaved.bak"),
             os.path.join(R, r"crates\liquid-verre\src\gfx\compositor.rs"))
print("probe exe archived; interleaved sources restored")
