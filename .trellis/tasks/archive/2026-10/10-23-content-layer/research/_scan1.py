import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
p_bmp = os.path.join(os.environ['TEMP'], 'verre_verify_content', 'iso_rgb.bmp.m0.bmp')
w, h, px = vc.read_bmp(p_bmp)
# 全图找纯灰 checker 像素（b=g=r ∈ {30,220}, a=255）
xs, ys = [], []
n = 0
for y in range(0, h, 3):
    for x in range(0, w, 3):
        p = px[y * w + x]
        if p[0] == p[1] == p[2] and p[0] in (30, 220) and p[3] == 255:
            xs.append(x); ys.append(y); n += 1
print("gray-checker pixels:", n)
if n:
    print("x range", min(xs), max(xs), " y range", min(ys), max(ys))
