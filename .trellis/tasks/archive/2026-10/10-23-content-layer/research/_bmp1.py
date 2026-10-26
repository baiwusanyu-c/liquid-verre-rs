import os
d = os.path.join(os.environ['TEMP'], 'verre_verify_content')
p = os.path.join(d, "iso_def.bmp.m0.bmp")
with open(p, "rb") as f:
    head = f.read(54)
import struct
filesize, _, _, dataoff = struct.unpack_from("<III", head, 2)
w, h, planes, bits = struct.unpack_from("<iIHH", head, 18)
print("filesize", filesize, "dataoff", dataoff, "w", w, "h(h)", h, "planes", planes, "bits", bits)
print("file actual size", os.path.getsize(p))
size = os.path.getsize(p)
with open(p, "rb") as f:
    f.seek(size - 4)
    last_px = f.read(4)
print("last 4 bytes:", last_px)
# 文件末尾像素（bottom-up 时 = 图像左上角；top-down 时 = 右下）
# 块区域左上角附近 (960+200, 660+200) 若 checker dark=30 → 期望 (30,30,30,255)
