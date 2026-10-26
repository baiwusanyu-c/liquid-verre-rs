import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
vc.EXE = r"D:\github-project\liquid-verre-rs\target\release\liquid-verre-playground.exe"
shot, log = vc.run_shot("iso_d5", "checker")
for l in log.splitlines():
    if "DIAG5" in l:
        print(l[:160])
w, h, px = vc.read_bmp(shot)
r = vc.parse_rects(log)["lower"]
print("sample:", px[(r[1] + 200) * w + (r[0] + 200)], "expect", vc.checker_expect(200, 200))
