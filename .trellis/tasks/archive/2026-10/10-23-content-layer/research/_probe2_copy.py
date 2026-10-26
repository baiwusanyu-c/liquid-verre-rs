import shutil, os, hashlib, datetime
R = r"D:\github-project\liquid-verre-rs"
research = os.path.join(R, ".trellis", "tasks", "10-23-content-layer", "research")
exe = os.path.join(R, r"target\release\liquid-verre-playground.exe")
probe = os.path.join(research, "vc_batch_probe.exe")
shutil.copy2(exe, probe)
# restore interleaved
shutil.copy2(os.path.join(research, "glass_renderer.interleaved.bak"), os.path.join(R, r"crates\liquid-verre\src\gfx\glass_renderer.rs"))
shutil.copy2(os.path.join(research, "compositor.interleaved.bak"), os.path.join(R, r"crates\liquid-verre\src\gfx\compositor.rs"))
os.utime(os.path.join(R, r"crates\liquid-verre\src\gfx\glass_renderer.rs"), None)
os.utime(os.path.join(R, r"crates\liquid-verre\src\gfx\compositor.rs"), None)
def sha(p):
    return hashlib.sha256(open(p, "rb").read()).hexdigest()[:12]
print("probe:", sha(probe), datetime.datetime.fromtimestamp(os.path.getmtime(probe)))
