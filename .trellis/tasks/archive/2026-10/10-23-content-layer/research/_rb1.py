import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
vc.EXE = r"D:\github-project\liquid-verre-rs\target\release\liquid-verre-playground.exe"
shot, log = vc.run_shot("iso_rb", "checker")
for l in log.splitlines():
    if "DIAG4" in l or "DIAG3" in l:
        print(l[:200])
