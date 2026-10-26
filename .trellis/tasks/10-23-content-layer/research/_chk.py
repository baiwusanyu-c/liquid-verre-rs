import os, datetime
R = r"D:\github-project\liquid-verre-rs"
def mt(p):
    return datetime.datetime.fromtimestamp(os.path.getmtime(p)).strftime("%H:%M:%S")
exe = os.path.join(R, r"target\release\liquid-verre-playground.exe")
research = os.path.join(R, ".trellis", "tasks", "10-23-content-layer", "research")
probe = os.path.join(research, "vc_batch_probe.exe")
print("target exe :", mt(exe), os.path.getsize(exe))
print("probe exe  :", mt(probe), os.path.getsize(probe))
g = open(os.path.join(R, r"crates\liquid-verre\src\gfx\glass_renderer.rs"), encoding="utf-8").read()
assert "draw_contents_batch" not in g, "batch method still in source!"
k = open(os.path.join(R, r"crates\liquid-verre\src\gfx\compositor.rs"), encoding="utf-8").read()
assert "draw_contents_batch" not in k and "Some(content_store)" in k, "compositor not interleaved!"
print("source is interleaved; probe archived separately")
