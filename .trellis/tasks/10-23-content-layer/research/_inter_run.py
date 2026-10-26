import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
vc.EXE = r"D:\github-project\liquid-verre-rs\target\release\liquid-verre-playground.exe"
os.environ["VC_DUMP"] = "1"
rc = vc.main()
print("INTERLEAVED exit code:", rc)
