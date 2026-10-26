import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
d = os.path.join(os.environ['TEMP'], 'verre_verify_content')
w, h, px = vc.read_bmp(os.path.join(d, "probe_checker_only.bmp.m0.bmp.overlay.bmp"))
print("overlay size", w, h)
for (sx, sy) in [(1560, 1040), (1012, 712), (1560, 1060)]:
    print("overlay", (sx, sy), px[sy * w + sx])
w2, h2, px2 = vc.read_bmp(os.path.join(d, "probe_checker_only.bmp.m0.bmp.frost.bmp"))
print("frost", (1560, 1040), px2[1040 * w2 + 1560])
