import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
d = os.path.join(os.environ['TEMP'], 'verre_verify_content')
for name in ("isorealsm.bmp.m0.bmp.overlay.bmp", "isorealsm.bmp.m0.bmp", "isorealsm.bmp.m0.bmp.capture.bmp"):
    w, h, px = vc.read_bmp(os.path.join(d, name))
    nz = sum(1 for p in px if p != (0, 0, 0, 0))
    nzsamp = sum(1 for y in range(660, 1420, 40) for x in range(960, 2160, 40) if px[y * w + x] != (0, 0, 0, 0))
    print(name, "nonzero:", nz, "blockarea-samples-nonzero:", nzsamp)
    if nz:
        for (sx, sy) in [(1560, 1040), (1012, 712), (1112, 768)]:
            print("   ", (sx, sy), px[sy * w + sx])
