import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
d = os.path.join(os.environ['TEMP'], 'verre_verify_content')
for f in sorted(os.listdir(d)):
    if f.startswith("checker") or f.startswith("isorealsm"):
        print(f, os.path.getsize(os.path.join(d, f)))
# 用 .m0 正确文件重测采样
w, h, px = vc.read_bmp(os.path.join(d, "isorealsm.bmp.m0.bmp"))
print("isorealsm .m0 composed:", px[1040 * w + 1560], px[712 * w + 1012])
