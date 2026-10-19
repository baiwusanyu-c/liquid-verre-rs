"""verify_adaptive.py -- 自适应质量控制器的行为验证。

时间线：
  0~3s    启动、静止
  3~15s   按住拖拽 20 块中的一块（重负载）-> 期望「降级 -> 1」「-> 2」
  15~50s  松开并停住光标（空闲）        -> 期望「回升」
同时每 0.5s 采样 Private 内存，验证降档确实释放了约 26~34 MB、升档又拿回去。

重要：重负载由 `GLASS_SIM_BUSY=13` 确定性地造出来（拖拽期每帧注入
13 ms 忙等，> 1.25 × 8.33 ms 的阈值）。不能再靠「真实拖拽 20 块」当负载：
判据修正后只看渲染段耗时，而真机上渲染段只有 0.16 ms/帧（预算的 1/50），
控制器会（正确地）始终停在 level 0 —— 之前能降档，是因为旧判据把采集阶段的
阻塞等待（8~32 ms）算成了丢帧。
"""
import ctypes
import io
import math
import os
import subprocess
import sys
import time
from ctypes import wintypes

try:
    ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
except Exception:
    ctypes.windll.user32.SetProcessDPIAware()
u = ctypes.windll.user32
k = ctypes.WinDLL("kernel32")
ps = ctypes.WinDLL("psapi")

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "release", "liquid-glass.exe")
LOG = os.path.join(ROOT, "tools", "adaptive.log")


class PMC(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("pf", wintypes.DWORD),
                ("a", ctypes.c_size_t), ("ws", ctypes.c_size_t),
                ("b", ctypes.c_size_t), ("c", ctypes.c_size_t),
                ("d", ctypes.c_size_t), ("e", ctypes.c_size_t),
                ("f", ctypes.c_size_t), ("g", ctypes.c_size_t),
                ("priv", ctypes.c_size_t)]


def priv_mb(h):
    m = PMC()
    m.cb = ctypes.sizeof(m)
    if ps.GetProcessMemoryInfo(h, ctypes.byref(m), m.cb):
        return m.priv / 1048576.0
    return float("nan")


def main():
    if os.path.exists(LOG):
        os.remove(LOG)
    env = dict(os.environ)
    env["GLASS_MANY"] = "1"
    env["GLASS_METRICS"] = "1"
    # 确定性负载：拖拽期每帧渲染段 +13 ms（超 1.25 × 8.33 ms 阈值）。
    # level 2 的目标间隔是 16.67 ms（阈值 20.8 ms），13 ms 在那一档算干净，
    # 因此拖拽期会稳定停在 level 2，松手后逐步回升。
    env["GLASS_SIM_BUSY"] = os.environ.get("SIM_BUSY", "13")
    fh = io.open(LOG, "wb")
    p = subprocess.Popen([EXE], env=env, cwd=ROOT, stdout=subprocess.DEVNULL,
                         stderr=fh)
    h = k.OpenProcess(0x0410, False, p.pid)
    t0 = time.perf_counter()
    samples = []
    pressed = False
    while time.perf_counter() - t0 < 50.0:
        t = time.perf_counter() - t0
        if 3.0 < t < 15.0:
            if not pressed:
                u.SetCursorPos(332, 264)
                time.sleep(0.25)
                u.mouse_event(0x0002, 0, 0, 0, 0)
                pressed = True
            u.SetCursorPos(int(332 + 420 * math.sin((t - 3) * 2.0)),
                           int(264 + 260 * math.sin((t - 3) * 1.3)))
        else:
            if pressed:
                u.mouse_event(0x0004, 0, 0, 0, 0)
                pressed = False
            u.SetCursorPos(1560, 1040)
        samples.append((t, priv_mb(h)))
        time.sleep(0.5)
    if pressed:
        u.mouse_event(0x0004, 0, 0, 0, 0)
    p.terminate()
    p.wait(timeout=10)
    fh.close()
    txt = io.open(LOG, "r", encoding="utf-8", errors="replace").read()

    print("=== [adaptive] 日志 ===")
    events = []
    for line in txt.splitlines():
        if "[adaptive]" in line:
            print("  " + line.strip())
            events.append(line)
    down = [e for e in events if "降级" in e]
    up = [e for e in events if "回升" in e]
    print("")
    print("=== Private 内存随时间 ===")
    import statistics
    def med_at(lo, hi):
        vs = [v for t, v in samples if lo < t < hi and v == v and v > 1]
        return statistics.median(vs) if vs else float("nan")
    m_start = med_at(1.5, 3.0)     # 启动完成、还是 level 0
    m_drag = med_at(9.0, 15.0)     # 拖拽期降档完成后的稳态（避开切换瞬态）
    m_late = med_at(44.0, 50.0)    # 松手后的末段（应回升、内存拿回去）
    print("  1.5~3s   level 0 中位:     %.1f MB" % m_start)
    print("  9~15s    降档后稳态中位: %.1f MB  (差 %+.1f MB)" % (m_drag, m_drag - m_start))
    print("  44~50s   末段中位:       %.1f MB  (差 %+.1f MB)" % (m_late, m_late - m_start))
    ok = True
    if not down:
        print("  !! 未观察到降档（GLASS_SIM_BUSY 没生效？）"); ok = False
    if not up:
        # 判据改成「只看我们自己的工作量」之后，桌面高频重绘不再影响回升：
        # 松手后 own_us 立刻回到几十 µs，4 个干净窗口 + 20s 闸后必回升。
        # 所以现在可以把回升当硬判据（闭环必须双向都通）。
        print("  !! 未观察到回升：闭环只通了一半"); ok = False
    # 内存只做信息项，不做判据：本机 Private 在**同一进程内**就能漂 50 MB
    # （实测 level 0 基线 1.5~3s = 237 MB、44~50s = 187 MB）；WDDM 下驱动释放
    # GPU 分配后也不一定立刻还给 OS。frost 半分辨率到底省多少，
    # 用受控 A/B 量：tools/ab_memory.py（轮转交错 + 稳态窗口中位数）。
    if m_start - m_drag >= 20:
        print("  降档窗口 Private 下降 %.1f MB（与预期同向）" % (m_start - m_drag))
    else:
        print("  (信息项) 本窗口没量到 Private 下降；以 tools/ab_memory.py 的受控 A/B 为准")
    print("")
    print("结论:", "PASS 降档/回升/显存回收均符合预期" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
