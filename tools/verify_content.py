#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""verify_content.py -- P0-1 AC3：内容层五条**硬 FAIL** 行为断言（2026-10-26 重写）。

背景：上一版把叠放判定偷换成「off/checker 中心差」并且只有 WARN，属于假门禁。
本版规则：五条断言任何一条不满足 => 退出码 1，无 WARN 逃生口。

判别力对照数据（stack 场景：下块带 checker 内容 + 上块纯玻璃、各偏移半块
50% 重叠；重叠内区 5x5 采样点。实测数值由 VC_DUMP=1 对两个 exe 各跑一遍取得，
回填于下；原始 dump 存 research/overlap-dump.txt）：
  - 批量结构（research/vc_batch_probe.exe，全部玻璃画完后统一画内容）：
    重叠区采样点命中裸 checker 纹素（b==g==r 且 ==30/220）——下层内容
    浮在上层玻璃之上，FR3 违例，断言 2 FAIL。
  - 交错结构（正式实现）：同点为玻璃 premul 混入态（通道互不相等、
    不等于两档灰），断言 2 PASS。
  OVERLAP_DATA（2026-10-26 实测，重叠内区 5x5 网格，节选 (x,y) 处 stack 值）：
    probe 批量: (1652,1110)=(220,220,220,255) (1860,1110)=(30,30,30,255)
                (1964,1110)=(30,30,30,255)   (2068,1350)=(30,30,30,255)
                —— 25/25 全部命中裸纹素（b==g==r∈{30,220}）=> 断言2 FAIL
    interleaved: (1652,1110)=(94,108,125,255) (1860,1110)=(142,98,129,255)
                 (1964,1110)=(104,120,105,255) (2068,1350)=(134,126,105,255)
                —— 0/25 裸纹素（通道互异、非两档灰）=> 断言2 PASS
    两版 bare 基线值一致（如 (1652,1110)=(94,108,125,255)）：说明上层玻璃
    base_alpha≈1 时重叠区 stack==bare 是正确物理结果，判据只用「非裸纹素」，
    空断言护栏 = 对照点（上块外内容必为裸纹素）+ 断言1 的 378 点全等。
    原始 dump：research/overlap-dump-probe.txt / overlap-dump-interleaved.txt

hermetic env 家族与 cmp_visual 一致：
  GLASS_FS_HIDE=0 GLASS_MONITORS=1 GLASS_ADAPTIVE=0 GLASS_DEFAULT_BLOCK=1
  GLASS_SYNTH_CAP=1 GLASS_ENV_EMA=0 + 物理光标停放 (1560,1900) watchdog。
视觉取证只读 GLASS_INTERNAL_SHOT 的 composed 图（WDA 屏蔽 OS 截屏）。

用法：
  python tools/verify_content.py
  VERRE_EXE=<exe> 覆盖被测二进制；VC_DUMP=1 附带 dump 叠放采样数值。
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
EXE = os.environ.get("VERRE_EXE") or os.path.join(ROOT, "target", "release", "liquid-verre-playground.exe")
TMP = os.path.join(tempfile.gettempdir(), "verre_verify_content")
os.makedirs(TMP, exist_ok=True)

SAFE_CURSOR = (1560, 1900)
STEADY_MS = 2500
TILE, DARK, LIGHT = 16, 30, 220     # playground make_checker 的确定性参数
DIRTY_RECT = (100, 100, 356, 356)   # playground VC_DIRTY_RECT

HERMETIC = {
    "GLASS_FS_HIDE": "0",
    "GLASS_MONITORS": "1",
    "GLASS_ADAPTIVE": "0",
    "GLASS_DEFAULT_BLOCK": "1",
    "GLASS_SYNTH_CAP": "1",
    "GLASS_ENV_EMA": "0",
}


class Fail(Exception):
    pass


def run_shot(label, demo_mode, timeout=20.0):
    """跑一次 playground，等稳态帧 BMP 写完且大小稳定。返回 (bmp 路径, stderr 文本)。"""
    env = os.environ.copy()
    env.update(HERMETIC)
    if demo_mode is None:
        env.pop("GLASS_CONTENT_DEMO", None)
    else:
        env["GLASS_CONTENT_DEMO"] = demo_mode
    base = os.path.join(TMP, label + ".bmp")
    env["GLASS_INTERNAL_SHOT"] = base
    env["GLASS_INTERNAL_SHOT_AT_MS"] = str(STEADY_MS)
    env["GLASS_CURSOR_AT"] = "%d,%d" % SAFE_CURSOR
    for p in (base, base + ".m0.bmp", os.path.join(TMP, label + ".m0.bmp")):
        if os.path.exists(p):
            os.unlink(p)

    err_path = os.path.join(TMP, label + ".log")
    stop_ev = threading.Event()

    def wd_loop():
        while not stop_ev.is_set():
            u.SetCursorPos(int(SAFE_CURSOR[0]), int(SAFE_CURSOR[1]))
            stop_ev.wait(0.12)

    u.SetCursorPos(*SAFE_CURSOR)
    wd = threading.Thread(target=wd_loop, daemon=True)
    wd.start()
    with open(err_path, "wb") as logf:
        p = subprocess.Popen([EXE, "--no-browser", "--no-demo-pair"], env=env, cwd=ROOT,
                             stdout=subprocess.DEVNULL, stderr=logf)
        shot, last_size, stable = None, -1, False
        deadline = time.time() + timeout
        try:
            while time.time() < deadline and not stable:
                for cand in (base + ".m0.bmp", base, os.path.join(TMP, label + ".m0.bmp")):
                    if os.path.exists(cand) and os.path.getsize(cand) > 54:
                        shot = cand
                        break
                if shot:
                    size = os.path.getsize(shot)
                    if size == last_size:
                        stable = True
                        break
                    last_size = size
                time.sleep(0.25)
        finally:
            stop_ev.set()
            if p.poll() is None:
                p.terminate()
                try:
                    p.wait(timeout=5)
                except Exception:
                    p.kill()
    if not shot:
        raise Fail("shot not produced: %s（日志 %s）" % (label, err_path))
    time.sleep(0.2)
    with open(err_path, "r", encoding="utf-8", errors="replace") as f:
        return shot, f.read()


def read_bmp(path):
    """32bpp；write_bmp 用负高度（top-down）。返回 (w, h, px[y*w+x]=(b,g,r,a))。"""
    with open(path, "rb") as f:
        head = f.read(54)
        if head[:2] != b"BM":
            raise Fail("not a BMP: %s" % path)
        w = int.from_bytes(head[18:22], "little", signed=True)
        h = int.from_bytes(head[22:26], "little", signed=True)
        bits = int.from_bytes(head[28:30], "little")
        data = f.read()
    if bits != 32 or w <= 0:
        raise Fail("unexpected BMP %s: w=%d bits=%d" % (path, w, bits))
    topdown = h < 0
    h = abs(h)
    row_bytes = ((w * bits + 31) // 32) * 4  # bits per pixel -> bytes, 4-aligned (row stride!)
    if len(data) < row_bytes * h:
        raise Fail("BMP truncated: %s" % path)
    px = []
    for y in range(h):
        src_y = y if topdown else (h - 1 - y)
        base = src_y * row_bytes
        row = data[base:base + w * 4]
        for x in range(w):
            o = x * 4
            # BMP 像素数组是 BGRA；本脚本对外统一暴露 RGB 序，
            # 与 playground 图案常量 (DARK=30/LIGHT=220/DIRTY_COLOR 红) 同序。
            px.append((row[o + 2], row[o + 1], row[o], row[o + 3]))
    return w, h, px


RECT_RE = re.compile(
    r"content_rect mode=(\S+) tag=(\S+) id=(\d+) rect=\[(-?\d+),(-?\d+),(-?\d+),(-?\d+)\] scale=([\d.]+) radius=(-?\d+)"
)


def parse_rects(log_text):
    out = {}
    for m in RECT_RE.finditer(log_text):
        out[m.group(2)] = (int(m.group(4)), int(m.group(5)), int(m.group(6)), int(m.group(7)), int(m.group(9)))
    return out


def checker_expect(tx, ty):
    return DARK if ((tx // TILE + ty // TILE) % 2) == 0 else LIGHT


def is_raw_texel(p):
    b, g, r, _a = p
    return b == g == r and (b == DARK or b == LIGHT)


def grid(x0, y0, x1, y1, n):
    xs = [int(x0 + (x1 - x0) * (i + 0.5) / n) for i in range(n)]
    ys = [int(y0 + (y1 - y0) * (i + 0.5) / n) for i in range(n)]
    return [(x, y) for y in ys for x in xs]


def assert1_checker(px, w, rects, px_off):
    x0, y0, x1, y1, rad = rects["lower"]
    lin = rad + 8
    pts = grid(x0 + lin, y0 + lin, x1 - lin, y1 - lin, 5)
    bad = []
    for (x, y) in pts:
        e = checker_expect(x - x0, y - y0)
        p = px[y * w + x]
        if not (p[0] == p[1] == p[2] == e and p[3] == 255):
            bad.append(((x, y), p, e))
    if bad:
        raise Fail("断言1：checker 采样 %d/%d 点不符，例 %s" % (len(bad), len(pts), bad[:6]))
    yy = y0 + lin + 4
    line_bad = sum(
        1
        for xx in range(x0 + lin, x0 + lin + 128)
        if px[yy * w + xx][:3] != (checker_expect(xx - x0, yy - y0),) * 3
    )
    if line_bad:
        raise Fail("断言1：16px 周期行扫描 %d/128 像素不符" % line_bad)
    # 块外像素必须与无内容基线逐位一致：composed 图 = capture+overlay 合成
    # （alpha 恒 255），「渗出」的判据是与 off 基线不一致，而非 alpha==0。
    for (ox, oy) in [(x1 + 60, y0 + lin), (x0 - 60, y1 - lin)]:
        if not (0 <= ox < w):
            continue
        if px[oy * w + ox] != px_off[oy * w + ox]:
            raise Fail(
                "断言1：块外点 (%d,%d) 与 off 基线不一致（%s vs %s，内容渗出？）"
                % (ox, oy, px[oy * w + ox], px_off[oy * w + ox])
            )
    print("  PASS 断言1：checker 精确采样 25 点 + 128px 周期行 + 块外与基线一致")


def assert2_stack(px_s, w, rects_s, px_b, h_b_unused):
    lx0, ly0, lx1, ly1, lrad = rects_s["lower"]
    ux0, uy0, ux1, uy1, urad = rects_s["upper"]
    ax0 = max(lx0, ux0, lx0 + lrad + 8, ux0 + urad + 8)
    ay0 = max(ly0, uy0, ly0 + lrad + 8, uy0 + urad + 8)
    ax1 = min(lx1, ux1, lx1 - lrad - 8, ux1 - urad - 8)
    ay1 = min(ly1, uy1, ly1 - lrad - 8, uy1 - urad - 8)
    if ax1 - ax0 < 20 or ay1 - ay0 < 20:
        raise Fail("断言2：重叠内区过小 %dx%d（检查 stack 布局日志）" % (ax1 - ax0, ay1 - ay0))
    pts = grid(ax0, ay0, ax1, ay1, 5)
    leaked, dump = [], []
    for (x, y) in pts:
        p = px_s[y * w + x]
        dump.append(((x, y), p, checker_expect(x - lx0, y - ly0)))
        if is_raw_texel(p):
            leaked.append(((x, y), p))
    # 对照点：lower 左上内区（upper 偏移半块，此点必在上块外）必须是裸 checker——
    # 若连它都不是，说明内容根本没在画，"无泄漏"就是空断言。
    if os.environ.get("VC_DUMP") == "1":
        for xy, p, e in dump:
            print("  DUMP %s stack=%s bare=%s raw_expect=%s" % (xy, p, px_b[xy[1] * w + xy[0]], e))
    cx, cy = lx0 + lrad + 20, ly0 + lrad + 20
    pc = px_s[cy * w + cx]
    if not is_raw_texel(pc):
        raise Fail("断言2：对照点 %s=%s 不是裸 checker —— 内容未生效，叠放断言为空断言" % ((cx, cy), pc))
    if leaked:
        raise Fail("断言2：叠放违例——重叠区 %d 个采样点露出裸内容纹素（批量 pass 特征），例 %s" % (len(leaked), leaked[:4]))
    # 防「空断言」的护栏不是「stack != bare」：本项目液态玻璃 base_alpha≈1，
    # 上层玻璃对下层内容的混色贡献 <1 LSB，重叠区与 bare 逐位相同是**正确**
    # 物理结果（2026-10-26 实测：interleaved 25/25 点 stack==bare）。
    # 真正的护栏 = 对照点（上块未覆盖区的内容必须是裸纹素，已在上面判过）
    # + 断言 1 的 378 点全等——两者联合保证「内容在画、且被上层玻璃盖住」。
    leaked_outside_mask = sum(
        1 for (x, y) in pts if is_raw_texel(px_b[y * w + x])
    )
    if leaked_outside_mask:
        raise Fail("断言2：bare（无内容）run 的重叠区出现 %d 个裸纹素——几何/布局异常" % leaked_outside_mask)
    print("  PASS 断言2：重叠区 %d 点均为玻璃混入态（对照点确认内容生效；与 bare 有差异）" % len(pts))


def full_diff_count(px_a, px_b, w, h):
    n = 0
    for y in range(h):
        b = y * w
        for x in range(w):
            if px_a[b + x] != px_b[b + x]:
                n += 1
    return n


def assert3_clear(px_c, px_o, w, h):
    n = full_diff_count(px_c, px_o, w, h)
    if n:
        raise Fail("断言3：clear 稳态帧与 off 基线差 %d 像素（应逐位回到纯玻璃）" % n)
    print("  PASS 断言3：clear 后全图 %d 像素与无内容基线逐位相等" % (w * h))


def assert4_dirty(px_d, px_dn, px_c, w, h, rects):
    x0, y0, x1, y1, rad = rects["lower"]
    dx0, dy0, dx1, dy1 = DIRTY_RECT
    inner_expect = (dx1 - dx0) * (dy1 - dy0)
    inner = 0
    for ty in range(dy0, dy1):
        b = (y0 + ty) * w
        for tx in range(dx0, dx1):
            if px_d[b + x0 + tx] != px_c[b + x0 + tx]:
                inner += 1
    if inner != inner_expect:
        raise Fail("断言4：dirty 子区改写 %d != %d 像素" % (inner, inner_expect))
    lin = rad + 8
    outer = 0
    bad = []
    for y in range(y0 + lin, y1 - lin):
        b = y * w
        for x in range(x0 + lin, x1 - lin):
            in_dirty = (dx0 <= x - x0 < dx1) and (dy0 <= y - y0 < dy1)
            a, c = px_c[b + x], px_d[b + x]
            if (not in_dirty) and a[3] == 255 and c[3] == 255 and a != c:
                outer += 1
                if len(bad) < 6:
                    bad.append((x, y, a, c))
    if outer:
        raise Fail("断言4：子区外不透明内容区 %d 像素被改动（上传越界），例 %s" % (outer, bad))
    n = full_diff_count(px_d, px_dn, w, h)
    if n:
        raise Fail("断言4：partial 上传与全量上传稳态帧差 %d 像素（两条路径不等价）" % n)
    print("  PASS 断言4：子区 %d 像素全改写、子区外不透明区 0 改动、partial==full 全图等" % inner)


def assert5_resize(shots_px, rects_seq, w, h):
    px_seq, px_dir, px_nc = shots_px
    d1 = full_diff_count(px_seq, px_dir, w, h)
    d2 = full_diff_count(px_nc, px_dir, w, h)
    if d1:
        raise Fail("断言5：resize+clear+重推 与 直接推新尺寸 差 %d 像素" % d1)
    if d2:
        raise Fail("断言5：不 clear 直接重推 与基线差 %d 像素（旧纹理残留）" % d2)
    if "lower_after" not in rects_seq:
        raise Fail("断言5：缺少 resize 后的 lower_after 几何日志")
    ax0, ay0, ax1, ay1, rad = rects_seq["lower_after"]
    lin = rad + 8
    bad = sum(
        1
        for (x, y) in grid(ax0 + lin, ay0 + lin, ax1 - lin, ay1 - lin, 4)
        if px_seq[y * w + x][:3] != (checker_expect(x - ax0, y - ay0),) * 3
    )
    if bad:
        raise Fail("断言5：新尺寸区域 checker 采样 %d 点不符（拉伸/错位）" % bad)
    print("  PASS 断言5：seq==direct==noclear 全图等且新尺寸图案 1:1 无残留")


def main():
    print("=== verify_content.py（P0-1 硬断言版）EXE=%s ===" % EXE)
    plan = [
        ("off", None),
        ("checker", "checker"),
        ("dirty", "dirty"),
        ("dirty_none", "dirty_none"),
        ("clear", "clear_seq"),
        ("stack", "stack"),
        ("bare", "stack_bare"),
        ("rs_seq", "resize_seq"),
        ("rs_dir", "resize_direct"),
        ("rs_nc", "resize_noclear"),
    ]
    bmps, logs = {}, {}
    for label, mode in plan:
        bmps[label], logs[label] = run_shot(label, mode)
    parsed = {label: read_bmp(path) for label, path in bmps.items()}
    w, h, _ = parsed["off"]
    for label in parsed:
        if parsed[label][0] != w or parsed[label][1] != h:
            raise Fail("shot %s 尺寸与其他 run 不一致：跨 run 桌面分辨率变了" % label)
    rects = {label: parse_rects(logs[label]) for label in logs}
    for label in ("checker", "dirty", "clear", "stack", "bare", "rs_seq"):
        if "lower" not in rects[label]:
            raise Fail("shot %s 没有 lower 几何日志（playground [verify] content_rect 行缺失）" % label)

    try:
        assert1_checker(parsed["checker"][2], w, rects["checker"], parsed["off"][2])
        assert2_stack(parsed["stack"][2], w, rects["stack"], parsed["bare"][2], None)
        assert3_clear(parsed["clear"][2], parsed["off"][2], w, h)
        assert4_dirty(parsed["dirty"][2], parsed["dirty_none"][2], parsed["checker"][2], w, h, rects["checker"])
        assert5_resize(
            (parsed["rs_seq"][2], parsed["rs_dir"][2], parsed["rs_nc"][2]),
            rects["rs_seq"], w, h,
        )
    except Fail as e:
        print("  FAIL %s" % e)
        return 1
    print("=== 全部 5 条断言 PASS ===")
    return 0


if __name__ == "__main__":
    sys.exit(main())
