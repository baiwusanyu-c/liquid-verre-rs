#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""verify_session_snapshot.py -- 0.5.2 #4：块几何变化 ⇒ 同帧快照可见（行为取证）。

主诉（宿主实测）：移动/缩放会话里库每帧写 b.x/b.y/b.width/b.height，但**不置**
`snapshot_dirty`，而快照发布门是 `snapshot_dirty || 250ms 兜底` —— 于是宿主只能按
~4 Hz 看到新矩形，表现为「图标/按钮比玻璃背景慢半拍」。宿主的临时绕行是
`set_default_resize` 催发布 + 16 ms tick 跟随。

判据（判别力设计）：
  playground 的 `GLASS_SESSION_DEMO=1` 起一条 **4 ms** 采样线程，只在
  `WindowSnapshot::moving == true`（= 库里正在跑移动会话）的采样点计数：
      ticks       = 会话内采样次数
      rect_changes= 其中 rect 与上一次采样不同的次数（= 快照里出现过的**不同**矩形数）
  脚本物理拖拽（SetCursorPos + mouse_event）后读 `[verify] session_snap ...`：
      断言 1  ticks >= 20                 —— 拖拽真的产生了会话（否则是空转测试）
      断言 2  rect_changes >= 0.6 * steps —— 每一次拖拽位移几乎都在快照里可见
      断言 3  会话必须在采样窗口内结束     —— 顺带覆盖 0.5.2 #4 的连带修正：
              `moving` 位若不进比对键，快照会永远停在 moving=true，
              采样线程等不到「会话结束」（2026-11-03 实测：修键之前
              ticks=3654 且只有 "(no session observed)" 收尾）。

  分母为什么用 steps 而不是 ticks：库里几何每**帧**才更新一次（120 Hz ≈ 8.3 ms），
  采样却是 4 ms —— 用 ticks 做分母时上限天然只有 ~0.5，测的是采样器而不是发布频率。
  反向对照依旧成立：退回 4 Hz 兜底档时，0.67 s 的会话只会发布 ~3 个不同矩形，
  远低于 0.6 × 40 = 24。

用法：
  python tools/verify_session_snapshot.py
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
    try:
        ctypes.windll.shcore.SetProcessDpiAwareness(2)
    except Exception:
        ctypes.windll.user32.SetProcessDPIAware()
u = ctypes.windll.user32

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")
TMP = os.path.join(tempfile.gettempdir(), "verre_session_snapshot")
os.makedirs(TMP, exist_ok=True)

MOUSEEVENTF_LEFTDOWN = 0x0002
MOUSEEVENTF_LEFTUP = 0x0004
ARMED = re.compile(r"session_demo armed id=(\d+) center=(-?\d+),(-?\d+)")
RESULT = re.compile(r"session_snap ticks=(\d+) rect_changes=(\d+)")

HERMETIC = {
    "GLASS_FS_HIDE": "0",
    "GLASS_MONITORS": "1",
    "GLASS_ADAPTIVE": "0",
    "GLASS_DEFAULT_BLOCK": "1",
    "GLASS_SYNTH_CAP": "1",
    "GLASS_ENV_EMA": "0",
    "GLASS_SESSION_DEMO": "1",
}


class Fail(Exception):
    pass


def read_log(path):
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def click(flag):
    u.mouse_event(flag, 0, 0, 0, 0)


def main():
    if not os.path.exists(EXE):
        print("FAIL  找不到被测二进制 %s（先 cargo build --release --workspace）" % EXE)
        return 1
    print("[session-snapshot] 被测 = %s" % EXE)
    env = dict(os.environ)
    env.update(HERMETIC)
    log_path = os.path.join(TMP, "run.log")
    with open(log_path, "wb") as logf:
        p = subprocess.Popen([EXE, "--no-browser", "--no-demo-pair"], env=env, cwd=ROOT,
                             stdout=subprocess.DEVNULL, stderr=logf)
    try:
        armed, deadline = None, time.time() + 20.0
        while time.time() < deadline and armed is None:
            m = ARMED.search(read_log(log_path))
            if m:
                armed = (int(m.group(2)), int(m.group(3)))
            time.sleep(0.1)
        if not armed:
            raise Fail("没等到 [verify] session_demo armed（日志 %s）" % log_path)
        cx, cy = armed
        print("  块中心 = %d,%d" % (cx, cy))
        time.sleep(1.0)  # 让采样线程进入稳态

        # ---- 物理拖拽：按下 -> 分步移动 -> 抬起 ----
        u.SetCursorPos(cx, cy)
        time.sleep(0.15)
        click(MOUSEEVENTF_LEFTDOWN)
        time.sleep(0.10)
        steps, dx, dy, gap = 40, 6, 4, 0.012
        for i in range(1, steps + 1):
            u.SetCursorPos(cx + i * dx, cy + i * dy)
            time.sleep(gap)
        time.sleep(0.10)
        click(MOUSEEVENTF_LEFTUP)
        print("  已物理拖拽 %d 步（每步 %+d/%+d px，间隔 %.0f ms）"
              % (steps, dx, dy, gap * 1000))

        res, deadline = None, time.time() + 15.0
        while time.time() < deadline and res is None:
            m = RESULT.search(read_log(log_path))
            if m:
                res = (int(m.group(1)), int(m.group(2)))
            time.sleep(0.1)
    finally:
        if p.poll() is None:
            p.terminate()
            try:
                p.wait(timeout=5)
            except Exception:
                p.kill()

    if not res:
        raise Fail("没等到 [verify] session_snap 结果行（日志 %s）" % log_path)
    ticks, changes = res
    need = int(0.6 * steps)
    print("  session_snap ticks=%d rect_changes=%d（拖拽 %d 步，要求 >= %d）"
          % (ticks, changes, steps, need))

    results = []
    ok1 = ticks >= 20
    print("  [1] 会话采样数 >= 20（拖拽真的发生了）-> %s" % ("PASS" if ok1 else "FAIL"))
    results.append(ok1)
    ok2 = changes >= need
    print("  [2] 每次拖拽位移几乎都在快照里可见（rect_changes=%d >= %d）-> %s"
          % (changes, need, "PASS" if ok2 else "FAIL"))
    results.append(ok2)
    ok3 = ticks < 2000
    print("  [3] 会话在采样窗口内结束（moving 位翻转被发布；ticks=%d < 2000）-> %s"
          % (ticks, "PASS" if ok3 else "FAIL"))
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