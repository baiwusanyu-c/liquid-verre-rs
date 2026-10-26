import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
vc.EXE = r"D:\github-project\liquid-verre-rs\target\release\liquid-verre-playground.exe"
shot, log = vc.run_shot("iso_diag2", "checker")
diag = [l[:200] for l in log.splitlines() if "DIAG" in l]
print("\n".join(diag[:8]))
print("diag lines:", len(diag))
