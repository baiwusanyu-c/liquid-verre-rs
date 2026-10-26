import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
# 去掉 SYNTH/ENV_EMA，回到 round-1 取证环境
vc.HERMETIC.pop("GLASS_SYNTH_CAP", None)
vc.HERMETIC.pop("GLASS_ENV_EMA", None)
vc.EXE = r"D:\github-project\liquid-verre-rs\target\release\liquid-verre-playground.exe"
shot, log = vc.run_shot("isorealsm", "checker")
w, h, px = vc.read_bmp(shot)
print("rects:", vc.parse_rects(log))
for (sx, sy) in [(1560, 1040), (1012, 712), (5, 5)]:
    print("real-desktop composed", (sx, sy), px[sy * w + sx])
err = [l for l in log.splitlines() if ("error" in l.lower() or "warn" in l.lower() or "fatal" in l.lower())]
print("errlines:", err[:6])
