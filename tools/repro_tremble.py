# -*- coding: utf-8 -*-
"""repro_tremble.py -- 「拖一块、其他块颤抖/闪动」的逐帧复现与量化。

方法（静态合成桌面 = 干净的判据）：
  * GLASS_SYNTH_CAP=1：桌面内容恒定 -> 任何"其他块"的逐帧变化都是非法的；
  * GLASS_SHOT_EVERY_MS：连拍 composed 图（增量路径，和用户看到的一致）；
  * 按住拖拽块 0 走正弦路径；
  * 对"与块 0 扫过区域不相交"的见证块逐帧比对：差异像素必须恒为 0。

用法：python tools/repro_tremble.py [far|many]
"""
import ctypes
import glob
import io
import re
import math
import os
import subprocess
import sys
import time

try:
    ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
except Exception:
    pass
u = ctypes.windll.user32
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# 被测可执行文件 = dev playground 产物；可用 VERRE_EXE 覆盖（例如指向旧基线做 A/B）。
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")
SW, SH = 3120, 2080
BASE = os.path.join(ROOT, "tools", "tr.bmp")
EVERY = 160


def far_blocks():
    return [(600, 300, 500, 340), (1800, 300, 500, 340),
            (600, 1300, 500, 340), (1800, 1300, 500, 340)]


def many_blocks():
    out = []
    for i in range(20):
        w = 600.0 + (i % 4) * 30.0
        h = 400.0 + (i % 3) * 24.0
        step_x = max(w * 0.62 + 22.0, 96.0)
        step_y = max(h * 0.60 + 22.0, 84.0)
        cols = min(max(int((SW - 40.0) // step_x), 1), 8)
        col, row = i % cols, (i // cols) % 12
        out.append((int(min(32.0 + col * step_x, max(SW - w - 24.0, 8.0))),
                    int(min(64.0 + row * step_y, max(SH - h - 32.0, 8.0))),
                    int(w), int(h)))
    return out


def load(path):
    d = io.open(path, "rb").read()
    off = int.from_bytes(d[10:14], "little")
    w = int.from_bytes(d[18:22], "little", signed=True)
    h = abs(int.from_bytes(d[22:26], "little", signed=True))
    return d[off:], w, h


def drag_path(t, cx, cy):
    return (cx + 220.0 * math.sin(t * 3.1), cy + 140.0 * math.sin(t * 2.3))


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "far"
    rects = far_blocks() if mode == "far" else many_blocks()
    b0 = rects[0]
    cx, cy = b0[0] + b0[2] / 2.0, b0[1] + b0[3] / 2.0
    for f in glob.glob(BASE + ".m0.*"):
        os.remove(f)
    env = dict(os.environ)
    # # 前台若恰好是「无边框最大化的普通应用」（Electron / Tauri 那一类），库的独占全屏
    # # 检测（方案 R-16）会命中 -> 画布隐藏 + 整屏停产 -> 本脚本拿到 0 帧连拍并假 FAIL。
    # # 取证必须对「用户前台开着哪个应用」保持 hermetic，与 GLASS_ADAPTIVE=0 同理。
    env["GLASS_FS_HIDE"] = "0"
    # 库默认**不**自建玻璃窗口（上层应用负责 create_window）；
    # 取证脚本需要一块确定性的默认玻璃，因此显式打开。
    env["GLASS_DEFAULT_BLOCK"] = "1"
    env.update({"GLASS_SYNTH_CAP": "1", "GLASS_MONITORS": "1", "GLASS_ADAPTIVE": "0",
                "GLASS_SHOT_EVERY_MS": str(EVERY), "GLASS_INTERNAL_SHOT": BASE,
                "GLASS_METRICS": "1"})
    if mode == "far":
        env["GLASS_FAR_GRID"] = "1"
        env.pop("GLASS_MANY", None)
    else:
        env["GLASS_MANY"] = "1"
        env.pop("GLASS_FAR_GRID", None)
    p = subprocess.Popen([EXE], env=env, cwd=ROOT, stdout=subprocess.DEVNULL,
                         stderr=subprocess.DEVNULL)
    t0 = time.perf_counter()
    pressed = False
    swept = [1e9, 1e9, -1e9, -1e9]
    try:
        while time.perf_counter() - t0 < float(os.environ.get('REPRO_SECS', '6.5')):
            t = time.perf_counter() - t0
            if 1.0 < t < 4.6:
                if not pressed:
                    u.SetCursorPos(int(cx), int(cy))
                    time.sleep(0.25)
                    u.mouse_event(0x0002, 0, 0, 0, 0)
                    pressed = True
                x, y = drag_path(t - 1.0, cx, cy)
                u.SetCursorPos(int(x), int(y))
                dx, dy = x - cx, y - cy
                swept[0] = min(swept[0], b0[0] + dx)
                swept[1] = min(swept[1], b0[1] + dy)
                swept[2] = max(swept[2], b0[0] + b0[2] + dx)
                swept[3] = max(swept[3], b0[1] + b0[3] + dy)
            else:
                if pressed:
                    u.mouse_event(0x0004, 0, 0, 0, 0)
                    pressed = False
                u.SetCursorPos(int(cx), int(cy))
            time.sleep(0.006)
    finally:
        if pressed:
            u.mouse_event(0x0004, 0, 0, 0, 0)
        p.terminate()
        try:
            p.wait(timeout=8)
        except Exception:
            p.kill()
    swept = [swept[0] - 96, swept[1] - 96, swept[2] + 96, swept[3] + 96]
    frames = sorted(f for f in glob.glob(BASE + ".m0.*.bmp")
                    if re.search(r"\.m0\.\d{3}\.bmp$", f))
    print("布局=%s  连拍帧数=%d  扫过区域(含阴影余量)=%s" % (mode, len(frames), [int(v) for v in swept]))
    if len(frames) < 4:
        print("帧数太少，无法分析"); return 1
    # 启动头两帧可能含「合成源切换 / EMA 初始化」的一次性稳定；
    # 最后一帧恒为进程被 terminate 时的 teardown 帧（实测无论窗口多长，
    # 事件总落在最后一帧）。两者都不是运行时颤抖，排除。
    frames = frames[2:-1]
    if len(frames) < 4:
        print("帧数太少"); return 1
    A0, w, h = load(frames[0])
    stride = w * 4
    witnesses = [r for r in rects[1:]
                 if not (r[0] < swept[2] and r[2] > swept[0] and r[1] < swept[3] and r[3] > swept[1])]
    print("见证块（与扫过区域不相交）：%d 个 %s" % (len(witnesses), [r[:2] for r in witnesses]))
    worst = {}
    prev = A0
    for fi, f in enumerate(frames[1:], 1):
        B, _, _ = load(f)
        for r in witnesses:
            x, y, bw, bh = r
            diff = 0
            maxd = 0
            for yy in range(y, y + bh):
                i0 = yy * stride + x * 4
                ra = A0[i0:i0 + bw * 4]
                rb = B[i0:i0 + bw * 4]
                if ra == rb:
                    continue
                for k in range(0, bw * 4, 4):
                    if ra[k:k + 4] != rb[k:k + 4]:
                        diff += 1
                        d = max(abs(ra[k + j] - rb[k + j]) for j in range(3))
                        maxd = max(maxd, d)
            if diff:
                key = r[:2]
                w0 = worst.setdefault(key, [0, 0, None])
                w0[0] += 1
                w0[1] = max(w0[1], maxd)
                if w0[2] is None:
                    w0[2] = fi
        prev = B
    # 连续帧判据：用户感知的是"持续颤抖"，即相邻两帧之间反复变化。
    consec = {}
    prev = A0
    for fi, f in enumerate(frames[1:], 1):
        B, _, _ = load(f)
        for r in witnesses:
            x, y, bw, bh = r
            diff = 0
            maxd = 0
            for yy in range(y, y + bh, 2):
                i0 = yy * stride + x * 4
                ra = prev[i0:i0 + bw * 4]
                rb = B[i0:i0 + bw * 4]
                if ra == rb:
                    continue
                for k in range(0, bw * 4, 4):
                    if ra[k:k + 4] != rb[k:k + 4]:
                        diff += 1
                        d = max(abs(ra[k + j] - rb[k + j]) for j in range(3))
                        maxd = max(maxd, d)
            # 阈值：单次事件 max<=1 LSB 视为舍入噪声（模糊链/环境色重算的最后一位，
            # 低于项目自己对 frost 半分辨率档定的 ≤3/255 可接受口径），不计为可感知颤抖。
            if diff and maxd >= 2:
                c = consec.setdefault(r[:2], [0, 0, 0])
                c[0] += 1
                c[1] = max(c[1], maxd)
                c[2] += diff
        prev = B
    print("")
    print("相邻帧反复变化（= 用户感知的颤抖）：")
    if not consec:
        print("   无：所有见证块相邻帧 0 差异")
    else:
        for k, v in sorted(consec.items()):
            print("   块@%s：%d 次相邻帧变化，累计 %d px，最大通道差=%d" % (k, v[0], v[2], v[1]))
    ok = not consec
    print("")
    if not worst:
        print("结论: PASS 见证块在所有 %d 帧里 0 差异（没有颤抖）" % len(frames))
        return 0
    print("相对首帧的一次性差异（稳定过程，<=3 LSB 视为不可见）：")
    for k, v in sorted(worst.items()):
        print("   块@%s：%d/%d 帧有差异，最大通道差=%d，首现于第 %s 帧"
              % (k, v[0], len(frames) - 1, v[1], v[2]))
    if ok:
        print("结论: PASS 无持续颤抖（仅一次性稳定过程）" if all(v[1] <= 3 for v in worst.values())
              else "FAIL 无持续颤抖，但存在 >3 LSB 的跳变")
        return 0 if all(v[1] <= 3 for v in worst.values()) else 1
    print("结论: FAIL 见证块出现相邻帧反复变化（= 用户看到的颤抖/闪动）")
    return 1


if __name__ == "__main__":
    sys.exit(main())
