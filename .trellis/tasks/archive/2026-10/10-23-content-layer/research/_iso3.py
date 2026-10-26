import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
d = os.path.join(os.environ['TEMP'], 'verre_verify_content')
for tag in ("off", "checker"):
    for suf in ("overlay", ""):
        p = os.path.join(d, "%s.bmp.m0.bmp%s" % (tag, (".%s.bmp" % suf) if suf else ""))
        w, h, px = vc.read_bmp(p)
        print(tag, suf or "composed", "center", px[1040 * w + 1560], "edge", px[700 * w + 1000])
