import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
vc.EXE = r"D:\github-project\liquid-verre-rs\target\release\liquid-verre-playground.exe"
shot_a, log_a = vc.run_shot("iso_exp3", "checker")
shot_b, log_b = vc.run_shot("iso_exp3_off", None)
wa, ha, pa = vc.read_bmp(shot_a)
wb, hb, pb = vc.read_bmp(shot_b)
diff = sum(1 for y in range(660, 1420, 11) for x in range(960, 2160, 17) if pa[y * wa + x] != pb[y * wb + x])
tot = len(range(660, 1420, 11)) * len(range(960, 2160, 17))
print("block-area diff checker-vs-off:", diff, "/", tot)
print("checker sample:", pa[1040 * wa + 1560], " off sample:", pb[1040 * wb + 1560])
