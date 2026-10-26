# -*- coding: utf-8 -*-
"""probe_click_through_timing.py -- 裁决实验：光标进入默认块后 TRANSPARENT 摘除时序。

背景：verify_focus_drop.py 报「WindowFromPoint 100% 漏检」，但其采样窗口（20ms×N）
短于主循环 idle tick（遮挡 B1 的 125ms 节拍即会把 runtime/mod.rs:774 的 click-through
判定跑一遍）。本实验回答三个决策问题：
  ① 进入块内后多久首次可命中（摘除延迟分布，是否需要消息驱动的自锁死）；
  ② 稳态命中率（撑住 OLE drag-over 的反复探测）；
  ③ 光标离开后是否恢复穿透（对照组，证明开关确实在动）。
"""
import ctypes, os, subprocess, sys, time

ROOT = "D:/github-project/liquid-verre-rs"
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")

u = ctypes.windll.user32
u.SetProcessDPIAware()

class POINT(ctypes.Structure):
    _fields_ = [("x", ctypes.c_long), ("y", ctypes.c_long)]

u.WindowFromPoint.restype = ctypes.c_void_p
u.GetClassNameW.restype = ctypes.c_int

SW = u.GetSystemMetrics(0); SH = u.GetSystemMetrics(1)
w = min(max(SW * 0.62, 720.0), 1200.0)
h = min(max(SH * 0.68, 480.0), 760.0)
BX = int(max((SW - w) * 0.5, 8.0)); BY = int(max((SH - h) * 0.5, 8.0))
BW = int(w); BH = int(h)

def hit(x, y):
    hw = u.WindowFromPoint(POINT(int(x), int(y))) or 0
    if not hw:
        return "NULL", 0
    buf = ctypes.create_unicode_buffer(64)
    u.GetClassNameW(hw, buf, 64)
    return buf.value, hw

def is_canvas(x, y):
    name, _ = hit(x, y)
    return name == "LiquidVerreCanvas"

def main():
    env = os.environ.copy()
    env.update({"GLASS_DEFAULT_BLOCK": "1", "GLASS_FS_HIDE": "0",
                "GLASS_MONITORS": "1", "GLASS_ADAPTIVE": "0"})
    u.SetCursorPos(5, SH - 5)
    time.sleep(0.4)
    proc = subprocess.Popen([EXE, "--no-browser", "--no-demo-pair"],
                            env=env, cwd=ROOT,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        time.sleep(3.0)  # boot 稳态
        cx, cy = BX + BW // 2, BY + BH // 2
        px, py = BX + 30, BY + BH // 3  # 块内另一个点（左上区域）
        u.SetCursorPos(5, SH - 5); time.sleep(0.8)
        # 对照 0：boot 全程光标在块外；若 idle 有重 attach，此时块中心应 miss
        miss_park = [is_canvas(cx, cy) for _ in range(4)]
        print("park 时块中心命中（期望全 False=穿透在生效）:", miss_park)

        first_hit = []
        stable_hits = []
        for r in range(10):
            t0 = time.perf_counter()
            u.SetCursorPos(cx, cy)
            dt = None
            for _ in range(300):          # 最多 1500ms，5ms 步进
                if is_canvas(cx, cy):
                    dt = (time.perf_counter() - t0) * 1000.0
                    break
                time.sleep(0.005)
            first_hit.append(dt)
            # 稳态：再连续 12 次、每 50ms 一次（模拟 OLE drag 的反复探测）
            sh = sum(1 for _ in range(12) if (time.sleep(0.05) or is_canvas(cx, cy)))
            stable_hits.append(sh)
            u.SetCursorPos(5, SH - 5)
            time.sleep(0.8)
            # 每轮里换第二个点，避免只测一个像素的运气
            if r == 4:
                cx, cy = px, py
        ok = [x for x in first_hit if x is not None]
        print("每轮首次命中延迟 ms:", [None if x is None else round(x, 1) for x in first_hit])
        print("每轮稳态命中（12 次采样）:", stable_hits)
        print(f"结果: 首轮命中率 {len(ok)}/10；最大摘除延迟 {round(max(ok),1) if ok else 'N/A'} ms；"
              f"稳态命中率 {sum(stable_hits)}/{len(stable_hits)*12}")
        return 0 if len(ok) >= 9 and sum(stable_hits) >= len(stable_hits) * 11 else 1
    finally:
        proc.terminate()
        try: proc.wait(4)
        except Exception: proc.kill()

if __name__ == "__main__":
    sys.exit(main())
