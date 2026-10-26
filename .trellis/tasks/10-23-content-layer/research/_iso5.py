import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
d = os.path.join(os.environ['TEMP'], 'verre_verify_content')
w, h, px = vc.read_bmp(os.path.join(d, "isorealsm.bmp.m0.bmp.overlay.bmp"))
print("overlay center", px[1040 * w + 1560], "edge", px[700 * w + 1000], "out", px[5 * w + 5])
