# -*- coding: utf-8 -*-
"""微验：一次按压后进程是否存活；stderr 尾部原样打出来。"""
import ctypes, os, subprocess, threading, time
ROOT = "D:/github-project/liquid-verre-rs"
EXE = os.path.join(ROOT, "target", "release", "liquid-verre-playground.exe")
u = ctypes.windll.user32
u.SetProcessDPIAware()
class PT(ctypes.Structure):
    _fields_ = [("x", ctypes.c_long), ("y", ctypes.c_long)]
u.WindowFromPoint.restype = ctypes.c_void_p
u.GetClassNameW.restype = ctypes.c_int
def canvas_at(x, y):
    hw = u.WindowFromPoint(PT(x, y)) or 0
    if not hw: return ""
    b = ctypes.create_unicode_buffer(64); u.GetClassNameW(hw, b, 64)
    return b.value
lines = []
env = os.environ.copy()
env.update({"GLASS_DEFAULT_BLOCK": "1", "GLASS_FS_HIDE": "0", "GLASS_MONITORS": "1",
            "GLASS_ADAPTIVE": "0", "GLASS_INPUT_SINK_DEMO": "1", "GLASS_FOCUS_DEMO": "1"})
p = subprocess.Popen([EXE, "--no-browser", "--no-demo-pair"], env=env, cwd=ROOT,
                     stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                     text=True, encoding="utf-8", errors="replace")
threading.Thread(target=lambda: [lines.append(l) for l in p.stderr], daemon=True).start()
time.sleep(3.0)
import re
rect = None
for l in reversed(lines):
    m = re.search(r"rect=\[(-?[\d.]+),\s*(-?[\d.]+),\s*(-?[\d.]+),\s*(-?[\d.]+)\] scale=([\d.]+)", l)
    if m: rect = [float(m.group(i)) for i in range(1, 6)]; break
print("rect:", rect)
x, y = int((rect[0] + rect[2]*0.25) * rect[4]), int((rect[1] + rect[3]*0.5) * rect[4])
u.SetCursorPos(x, y)
t0 = time.time()
while time.time() - t0 < 1.5 and canvas_at(x, y) != "LiquidVerreCanvas":
    time.sleep(0.005)
n0 = len(lines)
u.mouse_event(0x0002, 0, 0, 0, 0); time.sleep(0.3)
u.mouse_event(0x0004, 0, 0, 0, 0); time.sleep(1.0)
print("alive after press:", p.poll() is None, "exit=", p.poll())
print("---- 按压期全部新行 ----")
for l in lines[n0:]:
    if "sink_demo_state" in l: continue
    print(" ", l.rstrip())
p.terminate()
try: p.wait(3)
except Exception: p.kill()
