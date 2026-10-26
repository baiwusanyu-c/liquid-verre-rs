import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
vc.EXE = r"D:\github-project\liquid-verre-rs\target\release\liquid-verre-playground.exe"
shot, log = vc.run_shot("probe_checker_only", "checker")
w, h, px = vc.read_bmp(shot)
rects = vc.parse_rects(log)
print("rects:", rects)
x0, y0, x1, y1, rad = rects["lower"]
cx, cy = (x0 + x1) // 2, (y0 + y1) // 2
for (sx, sy) in [(cx, cy), (x0 + rad + 20, y0 + rad + 20), (x0 + 100 + 16, y0 + 100), (x0 + 600, y0 + 400)]:
    p = px[sy * w + sx]
    e = vc.checker_expect(sx - x0, sy - y0)
    print("sample", (sx, sy), "px=", p, "expect=", e, "RAW-OK" if p[0]==p[1]==p[2]==e and p[3]==255 else "MISMATCH")
