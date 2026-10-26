import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc

def sample(exe, label):
    vc.EXE = exe
    shot, log = vc.run_shot(label, "checker")
    w, h, px = vc.read_bmp(shot)
    r = vc.parse_rects(log)["lower"]
    x0, y0, x1, y1, rad = r
    pts = []
    for (tx, ty) in [(200, 200), (208, 200), (600, 400), (608, 400), (300, 300), (1000, 600)]:
        p = px[(y0 + ty) * w + (x0 + tx)]
        e = vc.checker_expect(tx, ty)
        pts.append(((tx, ty), p, e, "EXACT" if p[0] == p[1] == p[2] == e and p[3] == 255 else "off"))
    print(label, exe.split(os.sep)[-1])
    for t in pts:
        print("   ", t)

sample(r"D:\github-project\liquid-verre-rs\target\release\liquid-verre-playground.exe", "iso_interleaved")
sample(r"D:\github-project\liquid-verre-rs\.trellis\tasks\10-23-content-layer\research\vc_batch_probe.exe", "iso_probe")
