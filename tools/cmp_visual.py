# -*- coding: utf-8 -*-
"""cmp_visual.py -- 视觉回归取证（两问）

问题一：修复前(d406f93) vs 当前工作区，差异落在哪里？
  同一份确定性合成采集源 + 首帧取证图（光标固定在屏幕中心，两个二进制一致），
  按 interior（玻璃内部背景）/ ring（边框斜面环）/ outside（阴影余晖）三段统计。
  预期：只应有 ring 差异；interior 必须 0 差异。

问题二：当前构建里，光标在玻璃内部移动时，玻璃**内部背景**会不会变？
  同一构建、同一合成源，只改 GLASS_CURSOR_AT（两个都在块内的位置），
  再按同样三段统计。interior 出现差异 = 用户报的"背景颤抖"。

另外各拍一张**真实桌面**的 composed 图（不做逐像素比较，只供肉眼看过曝/对比度）。
"""
import ctypes
import io
import os
import struct
import subprocess
import sys
import time

try:
    ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
except Exception:
    pass
u = ctypes.windll.user32

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OLD = os.path.join(ROOT, "tools", "exe_old.exe")
NOW = os.path.join(ROOT, "tools", "exe_now.exe")
SW, SH = 3120, 2080
RING = 24


def single_rect():
    w = min(max(SW * 0.62, 720.0), 1200.0)
    h = min(max(SH * 0.68, 480.0), 760.0)
    return (int(max((SW - w) * 0.5, 8.0)), int(max((SH - h) * 0.5, 8.0)), int(w), int(h))


def many_rects():
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


def shot(exe, tag, many=False, cursor=None, synth=True, hold=9.0):
    base = os.path.join(ROOT, "tools", "cv_%s" % tag)
    env = dict(os.environ)
    env["GLASS_MONITORS"] = "1"
    env["GLASS_ADAPTIVE"] = "0"
    env["GLASS_INTERNAL_SHOT"] = base + ".bmp"
    env.pop("GLASS_INTERNAL_SHOT_AT_MS", None)
    if synth:
        env["GLASS_SYNTH_CAP"] = "1"
    else:
        env.pop("GLASS_SYNTH_CAP", None)
    if many:
        env["GLASS_MANY"] = "1"
    else:
        env.pop("GLASS_MANY", None)
    if cursor:
        env["GLASS_CURSOR_AT"] = "%d,%d" % cursor
    else:
        env.pop("GLASS_CURSOR_AT", None)
    p = subprocess.Popen([exe], env=env, cwd=ROOT, stdout=subprocess.DEVNULL,
                         stderr=subprocess.DEVNULL)
    time.sleep(hold)
    p.terminate()
    try:
        p.wait(timeout=8)
    except Exception:
        p.kill()
    time.sleep(0.5)
    f = base + ".bmp.m0.bmp"          # composed（玻璃叠在桌面上，= 用户看到的）
    assert os.path.exists(f), f
    return f


def load(path):
    d = io.open(path, "rb").read()
    off = int.from_bytes(d[10:14], "little")
    w = int.from_bytes(d[18:22], "little", signed=True)
    h = abs(int.from_bytes(d[22:26], "little", signed=True))
    return d[off:], w, h


def seg_stats(pa, pb, rects, label):
    A, w, h = load(pa)
    B, _, _ = load(pb)
    stride = w * 4
    acc = {"interior": [0, 0, 0], "ring": [0, 0, 0], "outside": [0, 0, 0]}
    for (bx, by, bw, bh) in rects:
        x0, x1 = max(0, bx - 60), min(w, bx + bw + 60)
        y0, y1 = max(0, by - 60), min(h, by + bh + 60)
        ix0, iy0, ix1, iy1 = bx + RING, by + RING, bx + bw - RING, by + bh - RING
        for yy in range(y0, y1):
            i0 = yy * stride
            ra, rb = A[i0 + x0 * 4:i0 + x1 * 4], B[i0 + x0 * 4:i0 + x1 * 4]
            if ra == rb:
                continue
            for xx in range(x0, x1):
                k = i0 + xx * 4
                a, b = A[k:k + 4], B[k:k + 4]
                if a == b:
                    continue
                d = max(abs(a[j] - b[j]) for j in range(3))
                in_box = bx <= xx < bx + bw and by <= yy < by + bh
                kind = "outside" if not in_box else (
                    "interior" if (ix0 <= xx < ix1 and iy0 <= yy < iy1) else "ring")
                s = acc[kind]
                s[0] += 1
                s[1] += d
                s[2] = max(s[2], d)
    print("  [%s]" % label)
    for k in ("interior", "ring", "outside"):
        c, s, m = acc[k]
        print("     %-9s 差异像素=%-9d 平均=%-6.2f 最大通道差=%d"
              % (k, c, (s / c if c else 0.0), m))
    return acc


print("=== 问题一：跨版本（合成源，光标都在屏幕中心）===")
r = {}
for tag, many in (("single", False), ("many", True)):
    rects = many_rects() if many else [single_rect()]
    a = shot(OLD, tag + "_old", many=many)
    b = shot(NOW, tag + "_now", many=many)
    print("-- %s（%s）" % (tag, "20 块密集" if many else "单块"))
    r[tag] = seg_stats(a, b, rects, "old vs now")

print("")
print("=== 问题二：当前构建，光标在玻璃内部两个不同位置（合成源）===")
sr = [single_rect()]
c1 = (sr[0][0] + 240, sr[0][1] + 240)
c2 = (sr[0][0] + sr[0][2] - 240, sr[0][1] + sr[0][3] - 240)
a = shot(NOW, "cur_in_a", cursor=c1)
b = shot(NOW, "cur_in_b", cursor=c2)
print("-- 单块，光标 %s vs %s（都在块内）" % (c1, c2))
r["cursor_inside"] = seg_stats(a, b, sr, "cursor A vs B")

print("")
print("=== 真实桌面 composed 图（供肉眼看过曝/对比度，不做逐像素比较）===")
ro = shot(OLD, "real_old", many=False, synth=False)
rn = shot(NOW, "real_now", many=False, synth=False)
print("  修复前:", ro)
print("  当前  :", rn)
print("")
print("=== 结论 ===")
for k, v in r.items():
    print("  %-14s interior=%d ring=%d outside=%d" %
          (k, v["interior"][0], v["ring"][0], v["outside"][0]))
