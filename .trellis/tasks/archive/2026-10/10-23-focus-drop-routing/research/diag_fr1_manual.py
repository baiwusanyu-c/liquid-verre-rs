# -*- coding: utf-8 -*-
"""FR1 手动诊断：起 demo 进程，看 attach 日志；停放光标+轮询+物理按压；dump 全部 [verify] 行。"""
import ctypes, os, re, subprocess, sys, threading, time

ROOT = "D:/github-project/liquid-verre-rs"
EXE = os.path.join(ROOT, "target", "release", "liquid-verre-playground.exe")
u = ctypes.windll.user32
u.SetProcessDPIAware()
SW = u.GetSystemMetrics(0); SH = u.GetSystemMetrics(1)

class POINT(ctypes.Structure):
    _fields_ = [("x", ctypes.c_long), ("y", ctypes.c_long)]
u.WindowFromPoint.restype = ctypes.c_void_p
u.GetClassNameW.restype = ctypes.c_int

def hit_name(x, y):
    hw = u.WindowFromPoint(POINT(int(x), int(y))) or 0
    if not hw: return "NULL", 0
    buf = ctypes.create_unicode_buffer(64)
    u.GetClassNameW(hw, buf, 64)
    return buf.value, hw

lines = []
def pump(pipe):
    for l in pipe:
        lines.append(l.rstrip())

env = os.environ.copy()
env.update({"GLASS_DEFAULT_BLOCK": "1", "GLASS_FS_HIDE": "0", "GLASS_MONITORS": "1",
            "GLASS_ADAPTIVE": "0", "GLASS_INPUT_SINK_DEMO": "1", "GLASS_FOCUS_DEMO": "1"})
proc = subprocess.Popen([EXE, "--no-browser", "--no-demo-pair"], env=env, cwd=ROOT,
                        stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                        text=True, encoding="utf-8", errors="replace")
threading.Thread(target=pump, args=(proc.stderr,), daemon=True).start()
try:
    time.sleep(3.5)
    print("---- 启动期 [verify] 行 ----")
    for l in lines:
        if "[verify]" in l or "sink" in l.lower() or "focus" in l.lower():
            print(" ", l)
    # 块矩形：从最近一条 sink_demo_state 心跳读实时值（不信公式估计——块可能被拖走/被用户动过）
    rect = None
    for l in reversed(lines):
        m = re.search(r'sink_demo_state id=\d+ rect=\[(-?[\d.]+),\s*(-?[\d.]+),\s*(-?[\d.]+),\s*(-?[\d.]+)\] scale=([\d.]+)', l)
        if m:
            rect = [float(m.group(i)) for i in range(1, 6)]
            break
    assert rect, "no state heartbeat"
    bx, by, w, h, sc = rect
    press_x, press_y = int((bx + w * 0.25) * sc), int((by + h * 0.5) * sc)
    print(f"块(实时)=({bx},{by},{int(w)},{int(h)}) 按压点=({press_x},{press_y})")
    u.SetCursorPos(press_x, press_y)
    ok = False
    for _ in range(200):
        nm, hw = hit_name(press_x, press_y)
        if nm == "LiquidVerreCanvas":
            ok = True; break
        time.sleep(0.005)
    print("按压点命中画布:", ok, "（轮询后）")
    for rnd in (1, 2):
        u.SetCursorPos(press_x, press_y)
        for _ in range(200):
            nm, _h = hit_name(press_x, press_y)
            if nm == "LiquidVerreCanvas":
                break
            time.sleep(0.005)
        time.sleep(0.15)
        n_before = len(lines)
        u.mouse_event(0x0002, 0, 0, 0, 0); time.sleep(0.25)
        u.mouse_event(0x0004, 0, 0, 0, 0); time.sleep(1.2)
        print(f"---- 第 {rnd} 次按压新增（全量转储）----")
        for l in lines[n_before:]:
            if "sink_demo_state" in l: continue
            print(" ", l)
        if not lines[n_before:]:
            print("  （无新增行）")
    print("---- 按压后新增 [verify] 行 ----")
    for l in lines[n_before:]:
        if "[verify]" in l: print(" ", l)
    allv = [l for l in lines if "[verify]" in l]
    print("全程 [verify] 行数:", len(allv))
finally:
    proc.terminate()
    try: proc.wait(4)
    except Exception: proc.kill()
