import io, os
R = r"D:\github-project\liquid-verre-rs"

# 1) fix read_bmp row_bytes
p = os.path.join(R, r"tools\verify_content.py")
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = "    row_bytes = ((w * 32 + 31) // 32) & ~3"
new = "    row_bytes = ((w * bits + 31) // 32) * 4  # bits per pixel -> bytes, 4-aligned (row stride!)"
assert old in c, "row_bytes line"
c = c.replace(old, new, 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("read_bmp row_bytes fixed")

# 2) confirm shaders on disk are the real ones
vs = io.open(os.path.join(R, r"crates\liquid-verre\shaders\content_vs.hlsl"), encoding="utf-8").read()
ps = io.open(os.path.join(R, r"crates\liquid-verre\shaders\content_ps.hlsl"), encoding="utf-8").read()
assert "SV_VertexID" in vs and "c0.xy" in vs and "EXPERIMENT" not in vs
assert "texContent" in ps and "SdRoundBox" in ps and "EXPERIMENT" not in ps
print("shader files are the real versions")

# 3) touch shaders so build.rs re-embeds
for n in ("content_vs.hlsl", "content_ps.hlsl", "border_vs.hlsl", "glass_ps.hlsl"):
    fp = os.path.join(R, r"crates\liquid-verre\shaders", n)
    os.utime(fp, None)
print("shaders touched")
