import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
d = os.path.join(os.environ['TEMP'], 'verre_verify_content')
w, h, px = vc.read_bmp(os.path.join(d, "isorealsm.bmp.m0.bmp"))
cx, cy = 1560, 1040
# 打印 8x8 邻域原始值
for y in range(cy - 4, cy + 4):
    row = []
    for x in range(cx - 4, cx + 4):
        p = px[y * w + x]
        row.append("%3d" % p[0])
    print(y, " ".join(row))
# 统计块内所有像素的 b 通道直方（top 6）
from collections import Counter
cnt = Counter()
for y in range(700, 1380, 7):
    for x in range(1000, 2120, 7):
        p = px[y * w + x]
        cnt[p] += 1
print("top:", cnt.most_common(6))
