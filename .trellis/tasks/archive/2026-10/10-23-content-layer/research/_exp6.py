import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
vc.EXE = r"D:\github-project\liquid-verre-rs\target\release\liquid-verre-playground.exe"
shot, log = vc.run_shot("iso_exp2", "checker")
w, h, px = vc.read_bmp(shot)
diag = [l[:120] for l in log.splitlines() if "DIAG" in l]
print("diag count:", len(diag), diag[:2])
for (x, y) in [(1560, 1040), (100, 100), (3000, 2000), (1560, 300)]:
    print((x, y), px[y * w + x])
