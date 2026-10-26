import sys, os
sys.path.insert(0, r"D:\github-project\liquid-verre-rs\tools")
import verify_content as vc
vc.EXE = r"D:\github-project\liquid-verre-rs\.trellis\tasks\10-23-content-layer\research\vc_batch_probe.exe"
os.environ["VC_DUMP"] = "1"
rc = vc.main()
print("PROBE exit code:", rc)
