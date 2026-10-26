# -*- coding: utf-8 -*-
"""verify_focus_drop.py v6 — P1 焦点路由与拖入命中取证（主会话重建版）。

FR3 拖入命中稳定性（WindowFromPoint，PID 归属判定 + 实时心跳矩形 + 前置轮询）：
  - 块内 9 采样点（四角内缩32px + 中心 + 四边中点）× ≥50 轮，每点 5ms 步进轮询
    ≤400ms；「稳定漏检」= 同点连续 3 个完整 400ms 窗口全 miss，断言 == 0。
  - 反向对照：块外 32px 四点的 miss 应占多数（证明穿透开关在动，防反向假绿）。
  - 启动前清扫孤儿 playground 实例（历史误判根因），命中判定按进程 pid 归属。
FR1 焦点（2026-10-26 改造：库不代调 SetFocus，发 EngineEvent::FocusHint）：
  - 单发：物理按压左半（Consumed 区）后 [event] FocusHint + playground 主线程
    focus_applied(ret≠0)；按压后 1.5s 内渲染心跳 ≥3 拍（防死锁红线）。
  - 连击 100：Down/Up 事件数 ≥90、心跳不断、[warn] ≤2、进程存活。
用法：python tools/verify_focus_drop.py [--only fr1|fr3|all]
"""
import ctypes
import io
import os
import re
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")

u = ctypes.windll.user32
u.SetProcessDPIAware()
SW = u.GetSystemMetrics(0)
SH = u.GetSystemMetrics(1)
LEFTDOWN, LEFTUP = 0x0002, 0x0004


class POINT(ctypes.Structure):
    _fields_ = [("x", ctypes.c_long), ("y", ctypes.c_long)]


u.WindowFromPoint.restype = ctypes.c_void_p
u.GetClassNameW.restype = ctypes.c_int
u.GetWindowThreadProcessId.restype = ctypes.c_uint
u.FindWindowExW.restype = ctypes.c_void_p


class LP(ctypes.Structure):
    _fields_ = [("x", ctypes.c_long), ("y", ctypes.c_long)]


def set_cursor(x, y):
    u.SetCursorPos(int(x), int(y))


def hit_info(x, y):
    hw = u.WindowFromPoint(POINT(int(x), int(y))) or 0
    if not hw:
        return "", 0
    buf = ctypes.create_unicode_buffer(64)
    u.GetClassNameW(hw, buf, 64)
    pid = ctypes.c_ulong(0)
    u.GetWindowThreadProcessId(hw, ctypes.byref(pid))
    return buf.value, pid.value


def is_my_canvas(x, y, pid):
    nm, p = hit_info(x, y)
    return nm == "LiquidVerreCanvas" and p == pid


def enum_canvas_windows():
    out = []
    hw = 0
    while True:
        hw = u.FindWindowExW(0, hw, "LiquidVerreCanvas", None)
        if not hw:
            break
        pid = ctypes.c_ulong(0)
        u.GetWindowThreadProcessId(hw, ctypes.byref(pid))
        out.append((hw, pid.value))
    return out


def sweep_foreign(exclude_pid=None):
    foreign = [pid for _h, pid in enum_canvas_windows() if pid != exclude_pid]
    foreign = sorted(set(foreign))
    if foreign:
        print(f"  清扫孤儿画布进程 pids={foreign}")
        for pid in foreign:
            try:
                subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
            except Exception:
                pass
        for _ in range(20):
            if not [p for _h, p in enum_canvas_windows() if p != exclude_pid]:
                return True
            time.sleep(0.25)
        return False
    return True


def base_env(extra=None):
    e = os.environ.copy()
    e.update({"GLASS_DEFAULT_BLOCK": "1", "GLASS_FS_HIDE": "0",
              "GLASS_MONITORS": "1", "GLASS_ADAPTIVE": "0"})
    if extra:
        e.update(extra)
    return e


def kill_proc(proc):
    proc.terminate()
    try:
        proc.wait(4)
    except Exception:
        proc.kill()
    try:
        proc.wait(3)
    except Exception:
        pass


STATE_RE = re.compile(
    r"sink_demo_state id=\d+ rect=\[(-?[\d.]+),\s*(-?[\d.]+),\s*(-?[\d.]+),\s*(-?[\d.]+)\] scale=([\d.]+)")


def last_rect(lines_iter):
    rect = None
    for l in lines_iter:
        m = STATE_RE.search(l)
        if m:
            rect = [float(m.group(i)) for i in range(1, 6)]
    return rect


def inner_points(rect):
    x0, y0, w, h, sc = rect
    pts = []
    inset = 32
    for fx, fy in ((inset, inset), (w - inset, inset), (inset, h - inset),
                   (w - inset, h - inset), (w * 0.5, h * 0.5),
                   (w * 0.5, inset), (w * 0.5, h - inset), (inset, h * 0.5),
                   (w - inset, h * 0.5)):
        pts.append((int((x0 + fx) * sc), int((y0 + fy) * sc)))
    return pts


def outer_points(rect):
    x0, y0, w, h, sc = rect
    m = 32
    return [(int((x0 - m) * sc), int((y0 + h * 0.5) * sc)),
            (int((x0 + w + m) * sc), int((y0 + h * 0.5) * sc)),
            (int((x0 + w * 0.5) * sc), int((y0 - m) * sc)),
            (int((x0 + w * 0.5) * sc), int((y0 + h + m) * sc))]


# ==================== FR3 ====================

def fr3(rounds=50):
    print("\n=== FR3 拖入命中稳定性（v6：PID 归属 + 实时矩形 + 400ms 前置轮询）===")
    if not sweep_foreign():
        print("  FAIL: 孤儿画布清不干净")
        return False, None
    set_cursor(5, SH - 5)
    time.sleep(0.3)
    fd, path = _tf.mkstemp(suffix=".log", prefix="fr3_")
    os.close(fd)
    logf = open(path, "w", encoding="utf-8")
    proc = subprocess.Popen([EXE, "--no-browser", "--no-demo-pair"],
                            env=base_env({"GLASS_INPUT_SINK_DEMO": "1"}),
                            cwd=ROOT, stdout=logf, stderr=subprocess.STDOUT)

    def read():
        logf.flush()
        with io.open(path, "r", encoding="utf-8", errors="replace") as f:
            return f.read().split("\n")

    try:
        t0 = time.time()
        rect = None
        while time.time() - t0 < 14.0:
            rect = last_rect(read())
            if rect:
                break
            time.sleep(0.2)
        if not rect:
            print("  FAIL: 心跳矩形不可见")
            return False, None
        print(f"  实时块矩形: {rect}")
        pts = inner_points(rect)
        hit_total = 0
        miss_windows = 0
        stable_miss = 0
        lat = []
        consec = [0] * len(pts)
        for r in range(rounds):
            if r and r % 10 == 0:
                nr = last_rect(read())
                if nr:
                    pts = inner_points(nr)
            for i, (x, y) in enumerate(pts):
                set_cursor(x, y)
                got = False
                dt0 = time.time()
                while time.time() - dt0 < 0.4:
                    if is_my_canvas(x, y, proc.pid):
                        got = True
                        lat.append((time.time() - dt0) * 1000.0)
                        break
                    time.sleep(0.005)
                if got:
                    hit_total += 1
                    consec[i] = 0
                else:
                    miss_windows += 1
                    consec[i] += 1
                    if consec[i] >= 3:
                        stable_miss += 1
            set_cursor(5, SH - 5)
            time.sleep(0.05)
        total = rounds * len(inner_points(rect))
        print(f"  块内命中: {hit_total}/{total}；miss 窗口 {miss_windows}；稳定漏检 {stable_miss}（要求 0）")
        if lat:
            lat.sort()
            print(f"  首次命中延迟 ms: min={lat[0]:.1f} 中位={lat[len(lat)//2]:.1f} max={lat[-1]:.1f}")
        # 反向对照：块外 32px 应大多 miss（穿透开关在动）
        om, oh = 0, 0
        for _ in range(10):
            for (x, y) in outer_points(rect):
                set_cursor(x, y)
                hit = False
                dt0 = time.time()
                while time.time() - dt0 < 0.4:
                    if is_my_canvas(x, y, proc.pid):
                        hit = True
                        break
                    time.sleep(0.005)
                oh += 1 if hit else 0
                om += 0 if hit else 1
        print(f"  反向对照（块外）: miss={om} hit={oh}（miss 应过半）")
        ok = stable_miss == 0 and hit_total == total and om > (om + oh) // 2
        print(f"  FR3: {'PASS' if ok else 'FAIL'}")
        return ok, rect
    finally:
        kill_proc(proc)
        logf.close()
        try:
            os.unlink(path)
        except Exception:
            pass


# ==================== FR1 ====================

def fr1(rect_hint=None):
    print("\n=== FR1 FocusHint 端到端（库不代调 SetFocus；含防死锁心跳 + 连击100）===")
    if not sweep_foreign():
        print("  FAIL: 孤儿画布清不干净")
        return False
    set_cursor(5, SH - 5)
    time.sleep(0.3)
    fd, path = _tf.mkstemp(suffix=".log", prefix="fr1_")
    os.close(fd)
    logf = open(path, "w", encoding="utf-8")
    proc = subprocess.Popen([EXE, "--no-browser", "--no-demo-pair"],
                            env=base_env({"GLASS_INPUT_SINK_DEMO": "1",
                                          "GLASS_FOCUS_DEMO": "1"}),
                            cwd=ROOT, stdout=logf, stderr=subprocess.STDOUT)

    def read():
        logf.flush()
        with io.open(path, "r", encoding="utf-8", errors="replace") as f:
            return f.read()

    def press_point():
        rect = last_rect(read().split("\n"))
        if not rect:
            return None, None
        x0, y0, w, h, sc = rect
        return int((x0 + w * 0.25) * sc), int((y0 + h * 0.5) * sc)

    try:
        t0 = time.time()
        while time.time() - t0 < 14.0 and "sink_demo_state" not in read():
            time.sleep(0.2)
        px, py = press_point()
        if px is None:
            print("  FAIL: 心跳不可见")
            return False
        set_cursor(px, py)
        hit_ok = False
        t0 = time.time()
        while time.time() - t0 < 1.0:
            if is_my_canvas(px, py, proc.pid):
                hit_ok = True
                break
            time.sleep(0.005)
        if not hit_ok:
            print("  FAIL: 按压点命中不了自家画布")
            return False
        time.sleep(0.15)
        base = read()
        u.mouse_event(LEFTDOWN, 0, 0, 0, 0)
        time.sleep(0.2)
        u.mouse_event(LEFTUP, 0, 0, 0, 0)
        time.sleep(1.5)
        seg = read()[len(base):]
        hint = [x for x in seg.split("\n") if "FocusHint" in x]
        applied = [x for x in seg.split("\n") if "focus_applied" in x]
        hb = sum(1 for x in seg.split("\n") if "sink_demo_state" in x)
        ret_ok = bool(applied) and re.search(r"ret=0x(?!0\b)[0-9a-f]{2,}", applied[-1]) is not None
        print(f"  FocusHint 行数={len(hint)} -> {'PASS' if hint else 'FAIL'}")
        print(f"  focus_applied ret≠0 -> {'PASS' if ret_ok else 'FAIL'}  {applied[-1].strip() if applied else '(无)'}")
        print(f"  按压后心跳 {hb} 拍（≥3=渲染线程没死锁） -> {'PASS' if hb >= 3 else 'FAIL'}")
        ok = bool(hint) and ret_ok and hb >= 3

        px, py = press_point()
        if px is None:
            return False
        set_cursor(px, py)
        time.sleep(0.2)
        base2len = len(read())
        for _ in range(100):
            u.mouse_event(LEFTDOWN, 0, 0, 0, 0)
            time.sleep(0.012)
            u.mouse_event(LEFTUP, 0, 0, 0, 0)
            time.sleep(0.012)
        time.sleep(2.0)
        seg2 = read()[base2len:]
        hb2 = sum(1 for x in seg2.split("\n") if "sink_demo_state" in x)
        downs = sum(1 for x in seg2.split("\n") if "phase=Down" in x)
        dbls = sum(1 for x in seg2.split("\n") if "phase=DoubleClick" in x)
        ups = sum(1 for x in seg2.split("\n") if "phase=Up(None)" in x)
        warns = sum(1 for x in seg2.split("\n") if "[warn]" in x)
        alive = proc.poll() is None
        # Win32 双击语义：连击时每个偶数次 Down 被合成 DoubleClick（其 Up 变
        # MBUTTONUP 丢弃）——24ms 间隔 100 连击 ≈ 50 Down + 50 DBLCLK 是系统行为，
        # 不是丢事件。按压事件总数 = Down + DoubleClick 才是正确判据。
        presses = downs + dbls
        burst_ok = alive and presses >= 90 and ups >= 45 and hb2 >= 4 and warns <= 2
        print(f"  连击100: Down={downs}+DBLCLK={dbls}={presses}(≥90) Up={ups}(≥45) 心跳={hb2}(≥4) warn={warns}(≤2) 进程活={alive}")
        print(f"  连击压力 -> {'PASS' if burst_ok else 'FAIL'}")
        ok = ok and burst_ok
        print(f"  FR1: {'PASS' if ok else 'FAIL'}")
        return ok
    finally:
        kill_proc(proc)
        logf.close()
        try:
            os.unlink(path)
        except Exception:
            pass


# ==================== main ====================

import tempfile as _tf


def main():
    print(f"verify_focus_drop.py v6")
    print(f"  EXE={EXE}  屏幕={SW}x{SH}  时间={time.strftime('%H:%M:%S')}")
    if not os.path.isfile(EXE):
        print("FAIL: 被测 EXE 不存在（先 cargo build --release）")
        sys.exit(1)

    only = "all"
    if "--only" in sys.argv:
        i = sys.argv.index("--only")
        if i + 1 < len(sys.argv):
            only = sys.argv[i + 1]

    results = []
    rect = None
    if only in ("all", "fr3"):
        try:
            r3, rect = fr3()
            results.append(("FR3 拖入命中", r3))
        except Exception as e:
            print(f"FR3 EXCEPTION: {e}")
            import traceback
            traceback.print_exc()
            results.append(("FR3 拖入命中", False))
    if only in ("all", "fr1"):
        try:
            results.append(("FR1 焦点路由", fr1(rect)))
        except Exception as e:
            print(f"FR1 EXCEPTION: {e}")
            import traceback
            traceback.print_exc()
            results.append(("FR1 焦点路由", False))

    print("\n" + "=" * 60)
    all_pass = True
    for name, p in results:
        print(f"  {name}: {'PASS' if p else 'FAIL'}")
        if not p:
            all_pass = False
    print(f"\n{'ALL PASS' if all_pass else 'SOME FAILED'}")
    sys.exit(0 if all_pass else 1)


if __name__ == "__main__":
    main()
