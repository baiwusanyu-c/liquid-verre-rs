import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
vc.EXE = r"D:\github-project\liquid-verre-rs\target\release\liquid-verre-playground.exe"
shot, log = vc.run_shot("iso_red", "checker")
w, h, px = vc.read_bmp(shot)
r = vc.parse_rects(log)["lower"]
x0, y0 = r[0], r[1]
from collections import Counter
cnt = Counter()
for ty in range(60, 700, 29):
    for tx in range(60, 1140, 41):
        cnt[px[(y0 + ty) * w + (x0 + tx)]] += 1
print("top:", cnt.most_common(4))
