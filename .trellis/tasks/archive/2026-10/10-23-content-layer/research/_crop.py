import sys, os, zlib, struct
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
d = os.path.join(os.environ['TEMP'], 'verre_verify_content')
w, h, px = vc.read_bmp(os.path.join(d, "isorealsm.bmp.m0.bmp"))
# 导出 [1400..1720]x[950..1130] 区域为 PNG（RGB）
x0, y0, x1, y1 = 1400, 950, 1720, 1130
cw, ch = x1 - x0, y1 - y0
raw = bytearray()
for y in range(y0, y1):
    raw.append(0)
    for x in range(x0, x1):
        b, g, r, a = px[y * w + x]
        raw += bytes((r, g, b))
def chunk(tag, data):
    return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xffffffff)
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", cw, ch, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(bytes(raw))) + chunk(b"IEND", b"")
out = os.path.join(d, "crop.png")
with open(out, "wb") as f:
    f.write(png)
print("wrote", out, cw, "x", ch)
