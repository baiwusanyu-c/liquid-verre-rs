#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""verify_border_zorder.py -- 0.5.2 #1：缩放反馈 border 必须受块 z 序遮挡（像素 oracle）。

主诉（宿主 ikun_organizer 实测）：两个分类框重叠时，拖动**下层**框的边缘缩放，
白色 border 会画到**上层**框身上。根因是 border 曾是 `draw_blocks` 之后的独立叠加
pass —— 对所有块的玻璃都是「后画 = 压在上面」，缺的是 border 层与玻璃层之间的顺序。

本版判据（GPU 像素级，`cargo xtask` 门禁之外，需物理桌面）：
  断言 1  fb=on  : p_out（下层框左边缘 border、上层框**之外**）必须是白
  断言 2  fb=on  : p_in （同一条 border、落在上层框**之内**）绝不能是白，且 == 上层框颜色
  断言 3  fb=off : p_out 不是白（反向对照：删掉 border 发射，断言 1 必须红）
  断言 4  跨运行 : 两次 p_in 颜色一致（几何稳定，排除「两张图根本不同场景」的空转）

场景由 playground 的 `GLASS_BORDER_DEMO=on|off` 构造，几何/采样点/光标位置全部由
playground 算好后打在 `[verify] border_demo ...` 日志里，脚本**跟着日志走**
（不在脚本里第二处手写布局，符合项目「单一来源」规矩）。

hermetic env 家族与 verify_content 一致：GLASS_FS_HIDE=0 GLASS_MONITORS=1
GLASS_ADAPTIVE=0 GLASS_DEFAULT_BLOCK=1 GLASS_SYNTH_CAP=1 GLASS_ENV_EMA=0。
视觉取证只读 GLASS_INTERNAL_SHOT 的 composed 图（WDA 屏蔽 OS 截屏）。

用法：
  python tools/verify_border_zorder.py
  VERRE_EXE=<exe> 覆盖被测二进制。
"""
import os
import re
import subprocess
import sys
import tempfile
import threading
import time

try:
    import ctypes
    ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
    u = ctypes.windll.user32
except Exception:
    u = None

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")
TMP = os.path.join(tempfile.gettempdir(), "verre_border_zorder")
os.makedirs(TMP, exist_ok=True)

STEADY_MS = 2500
HERMETIC = {
    "GLASS_FS_HIDE": "0",
    "GLASS_MONITORS": "1",
    "GLASS_ADAPTIVE": "0",
    "GLASS_DEFAULT_BLOCK": "1",
    "GLASS_SYNTH_CAP": "1",
    "GLASS_ENV_EMA": "0",
}

LINE = re.compile(
    r"border_demo fb=(?P<fb>\w+) a=\[(?P<a>[-\d.,]+)\] b=\[(?P<b>[-\d.,]+)\] "
    r"cursor=(?P<cx>[-\d.]+),(?P<cy>[-\d.]+) p_out=(?P<ox>[-\d.]+),(?P<oy>[-\d.]+) "
    r"p_in=(?P<ix>[-\d.]+),(?P<iy>[-\d.]+) color_b=(?P<cr>[-\d.]+),(?P<cg>[-\d.]+),(?P<cb>[-\d.]+)"
)


class Fail(Exception):
    pass


def parse_rect(text):
    return [float(v) for v in text.split(",")]


def run_case(fb_mode, timeout=45.0):
    """跑一次取证，返回 (bmp 路径, 场景参数 dict)。"""
    env = dict(os.environ)
    env.update(HERMETIC)
    env["GLASS_BORDER_DEMO"] = fb_mode
    base = os.path.join(TMP, "shot_%s.bmp" % fb_mode)
    env["GLASS_INTERNAL_SHOT"] = base
    env["GLASS_INTERNAL_SHOT_AT_MS"] = str(STEADY_MS)
    for p in (base, base + ".m0.bmp"):
        if os.path.exists(p):
            os.unlink(p)
    log_path = os.path.join(TMP, "log_%s.txt" % fb_mode)

    stop = threading.Event()
    holder = {}

    def watchdog():
        # 光标必须一直钉在「下层框左边缘命中带」里：hover 消失 => border 消失 => 假 FAIL。
        while not stop.is_set():
            pt = holder.get("cursor")
            if pt and u is not None:
                u.SetCursorPos(int(pt[0]), int(pt[1]))
            stop.wait(0.10)

    with open(log_path, "wb") as logf:
        p = subprocess.Popen([EXE, "--no-browser", "--no-demo-pair"], env=env, cwd=ROOT,
                             stdout=subprocess.DEVNULL, stderr=logf)
        wd = threading.Thread(target=watchdog, daemon=True)
        wd.start()
        shot, scene, deadline = None, None, time.time() + timeout
        try:
            while time.time() < deadline:
                if scene is None:
                    scene = read_scene(log_path)
                    if scene:
                        holder["cursor"] = (scene["cx"], scene["cy"])
                if scene:
                    cand = base + ".m0.bmp"
                    if os.path.exists(cand) and os.path.getsize(cand) > 54:
                        size = os.path.getsize(cand)
                        time.sleep(0.4)
                        if os.path.getsize(cand) == size:
                            shot = cand
                            break
                time.sleep(0.2)
        finally:
            stop.set()
            if p.poll() is None:
                p.terminate()
                try:
                    p.wait(timeout=5)
                except Exception:
                    p.kill()
            time.sleep(2.0)  # 释放 duplication / DComp，下一用例才拿得到设备
    if not scene:
        raise Fail("没解析到 [verify] border_demo 场景行（日志 %s）" % log_path)
    if not shot:
        raise Fail("没产出稳态帧 %s（日志 %s）" % (base + ".m0.bmp", log_path))
    return shot, scene


def read_scene(path):
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            for line in f:
                m = LINE.search(line)
                if m:
                    return m.groupdict()
    except OSError:
        pass
    return None


def load_bmp(path):
    with open(path, "rb") as f:
        head = f.read(54)
        if head[:2] != b"BM":
            raise Fail("not a BMP: %s" % path)
        w = int.from_bytes(head[18:22], "little", signed=True)
        h = int.from_bytes(head[22:26], "little", signed=True)
        bits = int.from_bytes(head[28:30], "little")
        data = f.read()
    if bits != 32 or w <= 0:
        raise Fail("unexpected BMP %s (w=%d bits=%d)" % (path, w, bits))
    # BMP 约定：**负高度 = top-down**（write_bmp 就是这么写的），正高度是 bottom-up。
    return data, w, abs(h), h < 0


def pixel(data, w, h, top_down, x, y):
    x = int(round(x))
    y = int(round(y))
    if not (0 <= x < w and 0 <= y < h):
        raise Fail("采样点越界 %d,%d（画布 %dx%d）" % (x, y, w, h))
    row = y if top_down else (h - 1 - y)
    i = row * w * 4 + x * 4
    b, g, r, a = data[i], data[i + 1], data[i + 2], data[i + 3]
    return (r, g, b, a)


def avg(data, w, h, top_down, x, y, box=1):
    """3x3 邻域均值：AA 边缘与 1px 抖动不该把判据打成运气游戏。

    采样点由 playground 钉在「离块边缘 3 逻辑像素」，正好落在 border 的全不透明
    芯里（band = [0, bw-aa) = [0, 4.5)）；box=1 的 3x3 窗不会越过 band 外沿。
    """
    acc = [0, 0, 0]
    n = 0
    for dy in range(-box, box + 1):
        for dx in range(-box, box + 1):
            px = pixel(data, w, h, top_down, x + dx, y + dy)
            acc[0] += px[0]
            acc[1] += px[1]
            acc[2] += px[2]
            n += 1
    return tuple(v / n for v in acc) + (255.0,)


def is_white(px, tol=15.0):
    return all(v >= 255.0 - tol for v in px[:3])


def close(a, b, tol=30.0):
    return all(abs(a[i] - b[i]) <= tol for i in range(3))


def main():
    if not os.path.exists(EXE):
        print("FAIL  找不到被测二进制 %s（先 cargo build --release --workspace）" % EXE)
        return 1
    print("[border-zorder] 被测 = %s" % EXE)
    results = []

    shot_on, sc_on = run_case("on")
    d_on, w, h, td = load_bmp(shot_on)
    shot_off, sc_off = run_case("off")
    d_off, w2, h2, td2 = load_bmp(shot_off)

    def pt(sc, kx, ky):
        return float(sc[kx]), float(sc[ky])

    out_on = avg(d_on, w, h, td, *pt(sc_on, "ox", "oy"))
    in_on = avg(d_on, w, h, td, *pt(sc_on, "ix", "iy"))
    out_off = avg(d_off, w, h, td2, *pt(sc_off, "ox", "oy"))
    in_off = avg(d_off, w, h, td2, *pt(sc_off, "ix", "iy"))
    b_color = (float(sc_on["cr"]), float(sc_on["cg"]), float(sc_on["cb"]))

    print("  场景 A=%s B=%s p_out=%s p_in=%s 上层色=%s"
          % (sc_on["a"], sc_on["b"], (sc_on["ox"], sc_on["oy"]),
             (sc_on["ix"], sc_on["iy"]), b_color))
    print("  像素 fb=on : p_out=%s p_in=%s" % (tuple(round(v) for v in out_on[:3]),
                                               tuple(round(v) for v in in_on[:3])))
    print("  像素 fb=off: p_out=%s p_in=%s" % (tuple(round(v) for v in out_off[:3]),
                                               tuple(round(v) for v in in_off[:3])))

    # 断言 1：没被遮挡的 border 必须是白（否则这条脚本根本没在测 border）
    ok1 = is_white(out_on)
    print("  [1] p_out(fb=on) 是白 -> %s" % ("PASS" if ok1 else "FAIL"))
    results.append(ok1)

    # 断言 2：被上层块遮住的 border 段绝不能是白，且必须 == 上层块颜色
    dist_white = 255.0 - min(in_on[:3])
    ok2 = (not is_white(in_on, tol=38.0)) and close(in_on, b_color, tol=40.0)
    print("  [2] p_in(fb=on) 非白（离白 %.0f）且等于上层块颜色 -> %s"
          % (dist_white, "PASS" if ok2 else "FAIL"))
    results.append(ok2)

    # 断言 3（反向对照 / 判别力）：关掉 feedback 后 p_out 不再是白，且与开的时候差异巨大
    delta = max(abs(out_on[i] - out_off[i]) for i in range(3))
    ok3 = (not is_white(out_off)) and delta >= 60.0
    print("  [3] 反向对照 p_out(fb=off) 非白 且 |on-off|=%.0f >= 60 -> %s"
          % (delta, "PASS" if ok3 else "FAIL"))
    results.append(ok3)

    # 断言 4：两次运行的 p_in 一致（场景稳定，排除「两张图不同布局」的空转）
    ok4 = close(in_on, in_off, tol=24.0)
    print("  [4] 跨运行 p_in 稳定 -> %s" % ("PASS" if ok4 else "FAIL"))
    results.append(ok4)

    bad = results.count(False)
    print("总体:", "ALL PASS" if bad == 0 else ("FAIL(%d/4)" % bad))
    return 0 if bad == 0 else 1


if __name__ == "__main__":
    sys.exit(main())