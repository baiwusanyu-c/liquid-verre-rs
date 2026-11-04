#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""verify_double_click.py -- 0.5.2 #5：画布窗口类必须带 CS_DBLCLKS，双击相位才可能到来。

前科：`register_class` 只给了 `CS_CLASSDC`，Windows 因此**从不**给画布发
`WM_LBUTTONDBLCLK`，`wnd_proc` 里的 `PointerPhase::DoubleClick` 分支是死路径。
宿主日志实锤：用户双击标题只得到两组独立 Down/Up，只能自己在 sink 里按
`GetDoubleClickTime()` + `SM_CX/CYDOUBLECLK` 合成双击（库补上后这段可退役）。

判据（带反向对照，删实现它还红吗）：
  用 `GLASS_INPUT_SINK_DEMO=1` 挂载 SinkDemo —— 它把每个到达 sink 的相位打成
  `[verify] sink_demo ... phase=<Phase>`。脚本物理点击（mouse_event）两次：
    慢速对（间隔 > GetDoubleClickTime + 150 ms）⇒ **不许**出现 DoubleClick
      （这一条排除「库自己合成相位」的假阳性：真有合成逻辑时这里必红）
    快速对（间隔 < GetDoubleClickTime / 2，同一坐标）⇒ **必须**出现 DoubleClick
  两条都过才算修好。

用法：
  python tools/verify_double_click.py
  VERRE_EXE=<exe> 覆盖被测二进制。
"""
import ctypes
import os
import re
import subprocess
import sys
import tempfile
import threading
import time

try:
    ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
except Exception:
    ctypes.windll.user32.SetProcessDPIAware()
u = ctypes.windll.user32

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")
TMP = os.path.join(tempfile.gettempdir(), "verre_double_click")
os.makedirs(TMP, exist_ok=True)

MOUSEEVENTF_LEFTDOWN = 0x0002
MOUSEEVENTF_LEFTUP = 0x0004
MOUNT = re.compile(r"sink_mount id=\d+ center=(-?\d+),(-?\d+)")

HERMETIC = {
    "GLASS_FS_HIDE": "0",
    "GLASS_MONITORS": "1",
    "GLASS_ADAPTIVE": "0",
    "GLASS_DEFAULT_BLOCK": "1",
    "GLASS_SYNTH_CAP": "1",
    "GLASS_ENV_EMA": "0",
    "GLASS_INPUT_SINK_DEMO": "1",
}


class Fail(Exception):
    pass


def read_log(path):
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def dbl_count(path):
    return read_log(path).count("phase=DoubleClick")


def click_pair(x, y, gap_s):
    """在 (x, y) 按两下左键，间隔 gap_s 秒。"""
    u.SetCursorPos(int(x), int(y))
    time.sleep(0.08)
    for _ in range(2):
        u.mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0)
        time.sleep(0.02)
        u.mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0)
        if gap_s:
            time.sleep(gap_s)


def main():
    if not os.path.exists(EXE):
        print("FAIL  找不到被测二进制 %s（先 cargo build --release --workspace）" % EXE)
        return 1
    print("[double-click] 被测 = %s" % EXE)
    dbl_time_ms = int(ctypes.windll.user32.GetDoubleClickTime())
    print("  系统 GetDoubleClickTime = %d ms" % dbl_time_ms)

    env = dict(os.environ)
    env.update(HERMETIC)
    log_path = os.path.join(TMP, "run.log")
    with open(log_path, "wb") as logf:
        p = subprocess.Popen([EXE, "--no-browser", "--no-demo-pair"], env=env, cwd=ROOT,
                             stdout=subprocess.DEVNULL, stderr=logf)
    stop = threading.Event()

    def watchdog(pt):
        while not stop.is_set():
            u.SetCursorPos(int(pt[0]), int(pt[1]))
            stop.wait(0.08)

    wd = None
    try:
        center, deadline = None, time.time() + 20.0
        while time.time() < deadline and center is None:
            m = MOUNT.search(read_log(log_path))
            if m:
                center = (int(m.group(1)), int(m.group(2)))
            time.sleep(0.1)
        if not center:
            raise Fail("没等到 [verify] sink_mount（日志 %s）" % log_path)
        print("  块中心 = %d,%d" % center)
        time.sleep(1.0)
        wd = threading.Thread(target=watchdog, args=(center,), daemon=True)
        wd.start()

        # ---- 反向对照：慢速两击不该产生 DoubleClick ----
        before = dbl_count(log_path)
        click_pair(center[0], center[1], (dbl_time_ms + 150) / 1000.0)
        time.sleep(0.6)
        slow_delta = dbl_count(log_path) - before
        downs_slow = read_log(log_path).count("phase=Down")

        # ---- 主判据：快速两击必须产生 DoubleClick ----
        before_fast = dbl_count(log_path)
        click_pair(center[0], center[1], (dbl_time_ms / 2.0) / 1000.0)
        time.sleep(0.6)
        fast_delta = dbl_count(log_path) - before_fast
        downs_total = read_log(log_path).count("phase=Down")
        dbl_total = dbl_count(log_path)
    finally:
        stop.set()
        if p.poll() is None:
            p.terminate()
            try:
                p.wait(timeout=5)
            except Exception:
                p.kill()

    print("  慢速对：新增 DoubleClick=%d（Down 计数=%d）" % (slow_delta, downs_slow))
    print("  快速对：新增 DoubleClick=%d（Down 总计数=%d）" % (fast_delta, downs_total))

    results = []
    # 4 次「按下」= Down + DoubleClick。Windows 的双击序列是 Down/Up/**DBLCLK**/Up，
    # 第二次按下以 WM_LBUTTONDBLCLK 到达，**不再**产生 Down —— 只数 Down 会把
    # 「修好了」判成「点击没送达」（2026-11-03 首跑实测：Down=3、DoubleClick=1）。
    presses = downs_total + dbl_total
    ok1 = presses >= 4
    print("  [1] sink 真的收到了 4 次按下（Down+DoubleClick=%d >= 4）-> %s"
          % (presses, "PASS" if ok1 else "FAIL"))
    results.append(ok1)
    ok2 = slow_delta == 0
    print("  [2] 慢速两击**不**产生 DoubleClick（排除库侧合成）-> %s"
          % ("PASS" if ok2 else "FAIL"))
    results.append(ok2)
    ok3 = fast_delta >= 1
    print("  [3] 快速两击产生 DoubleClick（CS_DBLCLKS 生效）-> %s"
          % ("PASS" if ok3 else "FAIL"))
    results.append(ok3)

    bad = results.count(False)
    print("总体:", "ALL PASS" if bad == 0 else "FAIL(%d/3)" % bad)
    return 0 if bad == 0 else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Fail as e:
        print("FAIL  %s" % e)
        sys.exit(1)