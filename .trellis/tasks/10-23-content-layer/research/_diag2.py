import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
vc.EXE = r"D:\github-project\liquid-verre-rs\target\release\liquid-verre-playground.exe"
shot, log = vc.run_shot("iso_diag", "checker")
for l in log.splitlines():
    if "DIAG" in l or "content" in l.lower():
        print(l[:200])
w, h, px = vc.read_bmp(shot)
r = vc.parse_rects(log)["lower"]
x0, y0 = r[0], r[1]
print("sample(200,200):", px[(y0 + 200) * w + (x0 + 200)], "expect", vc.checker_expect(200, 200))
