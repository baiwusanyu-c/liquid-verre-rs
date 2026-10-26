import os, struct
d = os.path.join(os.environ['TEMP'], 'verre_verify_content')
p = os.path.join(d, "iso_def.bmp.m0.bmp")
with open(p, "rb") as f:
    head = f.read(54)
filesize = struct.unpack_from("<I", head, 2)[0]
dataoff = struct.unpack_from("<I", head, 10)[0]
w = struct.unpack_from("<i", head, 18)[0]
hh = struct.unpack_from("<i", head, 22)[0]
bits = struct.unpack_from("<H", head, 28)[0]
comp = struct.unpack_from("<I", head, 30)[0]
print("filesize", filesize, "dataoff", dataoff, "w", w, "h", hh, "bits", bits, "comp", comp)
size = os.path.getsize(p)
print("actual size", size, "expected", 54 + abs(w) * abs(hh) * 4)
with open(p, "rb") as f:
    f.seek(size - 4)
    tail = f.read(4)
    f.seek(dataoff)
    first = f.read(4)
print("first px @dataoff:", tuple(first), " last px:", tuple(tail))
# 若 dataoff=54 且文件 = 54+w*h*4，则 top-down。
# 块左上 (960,660)+200 = (1160,860) 的像素：
row = ((w * bits + 31) // 32) * 4 // 4
with open(p, "rb") as f:
    f.seek(dataoff + 860 * row * 4 + 1160 * 4)
    px = f.read(4)
print("(1160,860):", tuple(px), "expect checker tx=200,ty=200 ->", 30 if ((200//16+200//16)%2==0) else 220)
