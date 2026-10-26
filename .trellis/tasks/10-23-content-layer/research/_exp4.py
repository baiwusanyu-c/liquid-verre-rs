import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
vc.EXE = r"D:\github-project\liquid-verre-rs\target\release\liquid-verre-playground.exe"
shot, log = vc.run_shot("iso_exp", "checker")
w, h, px = vc.read_bmp(shot)
from collections import Counter
cnt = Counter()
for y in range(700, 1400, 23):
    for x in range(1000, 2100, 31):
        cnt[px[y * w + x]] += 1
print("block area top:", cnt.most_common(3))
print("corner (1000,700):", px[700 * w + 1000])
