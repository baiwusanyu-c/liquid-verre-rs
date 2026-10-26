import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
vc.STEADY_MS = 6000
vc.EXE = r"D:\github-project\liquid-verre-rs\target\release\liquid-verre-playground.exe"
shot, log = vc.run_shot("iso_slow", "checker", timeout=30)
w, h, px = vc.read_bmp(shot)
r = vc.parse_rects(log)["lower"]
x0, y0 = r[0], r[1]
ok = tot = 0
for ty in range(60, 700, 37):
    for tx in range(60, 1140, 53):
        p = px[(y0 + ty) * w + (x0 + tx)]
        e = vc.checker_expect(tx, ty)
        tot += 1
        if p[0] == p[1] == p[2] == e and p[3] == 255:
            ok += 1
print("exact checker: %d/%d" % (ok, tot))
diag = [l[:100] for l in log.splitlines() if "DIAG" in l]
print(diag[:4])
