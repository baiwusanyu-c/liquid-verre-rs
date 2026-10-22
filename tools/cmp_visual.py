# -*- coding: utf-8 -*-
"""cmp_visual.py -- 视觉回归取证（三问）

问题一（跨版本，**稳态帧**）：重构前 vs 当前，同一份确定性合成采集源、同一光标位置，
  按 deep / interior / ring / outside 四段统计。**四段都必须 0 差异**。

问题二（光标移动，当前构建）：光标在玻璃「深内部」两点之间移动，
  deep 段必须 0 差异 —— 证明鼠标移动不会让玻璃里的**背景**颤抖；
  斜面环（bevel）里的高光/描边响应是设计意图，不计入判据。

问题三：各拍一张真实桌面的 composed 图，供肉眼看过曝 / 对比度（不做逐像素比较）。

—— 为什么比「稳态帧」而不是「首帧」——
重构前 `src/ui/panel.rs` 在主循环里每帧把参数写回 `app.params`，而它用
`Arc<AtomicIsize>` 存值，于是 `tint_opacity` 0.018 被量化截断成 0.01、
`blur_min` 被面板派生成 blur_max*0.12 = 2.4。也就是说旧二进制的**引导帧**
（面板 apply 之前）与**稳态帧**（之后）参数不同 —— 实测两者相差 873748 像素、
最大通道差 140。用户长期看到的是稳态帧，所以取证以稳态帧为准
（`GLASS_INTERNAL_SHOT_AT_MS=2500`）。库化之后不再有量化往返，两帧同一份参数。

—— 光标如何固定 ——
两条一起用：
  1. `GLASS_CURSOR_AT` 钉死（重构后全程生效；旧二进制只拿它做初值）；
  2. `SetCursorPos` **物理停放** + 看门狗线程持续复位（对旧二进制唯一有效的办法）。
停放点必须离任何块的边 >= 3*mouse_radius（默认 660px），这样鼠标点光的有限支撑窗
`gate` 精确为 0，两个二进制都不会画任何 hover 相关效果 —— 于是结果与
「跑测试时鼠标碰巧停在哪」无关。这一点也是实测踩出来的：停在块内时，
重构后新增的边缘/对角缩放 border 会让整片圆角对不上（最大通道差 140）。
"""
import ctypes
import io
import os
import subprocess
import sys
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
OLD = os.environ.get("VERRE_OLD_EXE") or os.path.join(ROOT, "tools", "exe_old.exe")
NOW = os.environ.get("VERRE_EXE") or os.path.join(ROOT, "tools", "exe_now.exe")
SW, SH = 3120, 2080
# ring = 边框斜面环的粗略宽度；deep = 完全超出斜面影响范围的内部。
# bevelWidth = lerp(26, min(hs), lens_coverage=0.15) ~= 79px，
# edgeWeight 的支撑是 bevelWidth*1.35 ~= 107px，取 130 留余量。
RING = 24
DEEP = 130
# 稳态帧延迟：必须 > 面板 apply 的第一帧，也要 > 环境色 EMA 的收敛时间（τ=150ms）。
STEADY_MS = 2500
# 光标停放点：主屏下方空白区，离单块玻璃与 20 块密集布局都 > 3*mouse_radius。
SAFE_CURSOR = (1560, 1900)


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


class _POINT(ctypes.Structure):
    _fields_ = [("x", ctypes.c_long), ("y", ctypes.c_long)]


def cursor_pos():
    p = _POINT()
    u.GetCursorPos(ctypes.byref(p))
    return (p.x, p.y)


class CursorWatchdog:
    """持续把真实光标按在指定点上（旧二进制不认 GLASS_CURSOR_AT，只能这么办）。

    会短暂抢走鼠标控制权，因此只在跨版本取证期间开启，并在退出时立刻停。
    """

    def __init__(self, xy, interval=0.12):
        self.xy = xy
        self.interval = interval
        self._stop = threading.Event()
        self._t = None

    def __enter__(self):
        u.SetCursorPos(int(self.xy[0]), int(self.xy[1]))

        def loop():
            while not self._stop.is_set():
                u.SetCursorPos(int(self.xy[0]), int(self.xy[1]))
                self._stop.wait(self.interval)

        self._t = threading.Thread(target=loop, daemon=True)
        self._t.start()
        return self

    def __exit__(self, *exc):
        self._stop.set()
        if self._t:
            self._t.join(timeout=1.0)
        return False


def shot(exe, tag, many=False, delay_ms=0, pin_cursor=None, synth=True, hold=9.0, watchdog=None):
    base = os.path.join(ROOT, "tools", "cv_%s" % tag)
    env = dict(os.environ)
    # # 前台若恰好是「无边框最大化的普通应用」（Electron / Tauri 那一类），库的独占全屏
    # # 检测（方案 R-16）会命中 -> 画布隐藏 + 整屏停产 -> 本脚本拿到 0 帧连拍并假 FAIL。
    # # 取证必须对「用户前台开着哪个应用」保持 hermetic，与 GLASS_ADAPTIVE=0 同理。
    env["GLASS_FS_HIDE"] = "0"
    # 库默认**不**自建玻璃窗口（那是上层应用的职责）；取证脚本需要一块确定性
    # 的默认玻璃，因此显式打开。
    env["GLASS_DEFAULT_BLOCK"] = "1"
    env["GLASS_MONITORS"] = "1"
    env["GLASS_ADAPTIVE"] = "0"
    env["GLASS_INTERNAL_SHOT"] = base + ".bmp"
    if delay_ms:
        env["GLASS_INTERNAL_SHOT_AT_MS"] = str(delay_ms)
    else:
        env.pop("GLASS_INTERNAL_SHOT_AT_MS", None)
    if pin_cursor:
        env["GLASS_CURSOR_AT"] = "%d,%d" % pin_cursor
    else:
        env.pop("GLASS_CURSOR_AT", None)
    if synth:
        env["GLASS_SYNTH_CAP"] = "1"
    else:
        env.pop("GLASS_SYNTH_CAP", None)
    # 像素等价比较必须冻掉一切**时间依赖**项。合成源只替换了采集纹理，
    # `changed` 仍来自真实桌面的 DDA 元数据：桌面一活跃，环境色 EMA 就会按
    # 墙钟 dt 推进（alpha = 1-exp(-dt/tau)），两次跑的推进时刻不同 => 全图 ±1 LSB
    # 漂移（实测 single deep 段 449582 像素 / 最大 2，且 now-vs-now 也不再自洽）。
    # GLASS_ENV_EMA=0 时 env_rt() 直接退回 1/16 层（确定性），EMA 本身的等价性
    # 由 verify_env_ema.py 在首帧单独证明（首帧 alpha 强制 1，与开关逐位一致）。
    env["GLASS_ENV_EMA"] = "0"
    if many:
        env["GLASS_MANY"] = "1"
    else:
        env.pop("GLASS_MANY", None)

    wd = CursorWatchdog(watchdog) if watchdog else None
    if wd:
        wd.__enter__()
    try:
        p = subprocess.Popen([exe], env=env, cwd=ROOT, stdout=subprocess.DEVNULL,
                             stderr=subprocess.DEVNULL)
        time.sleep(hold)
        p.terminate()
        try:
            p.wait(timeout=8)
        except Exception:
            p.kill()
    finally:
        if wd:
            wd.__exit__()
    time.sleep(0.4)
    f = base + ".bmp.m0.bmp"          # composed（玻璃叠在桌面上，= 用户看到的）
    assert os.path.exists(f), f
    return f


def load(path):
    d = io.open(path, "rb").read()
    off = int.from_bytes(d[10:14], "little")
    w = int.from_bytes(d[18:22], "little", signed=True)
    h = abs(int.from_bytes(d[22:26], "little", signed=True))
    return d[off:], w, h


SEGS = ("deep", "interior", "ring", "outside")


def seg_stats(pa, pb, rects, label):
    A, w, h = load(pa)
    B, _, _ = load(pb)
    stride = w * 4
    acc = {k: [0, 0, 0] for k in SEGS}
    for (bx, by, bw, bh) in rects:
        x0, x1 = max(0, bx - 60), min(w, bx + bw + 60)
        y0, y1 = max(0, by - 60), min(h, by + bh + 60)
        dx0, dy0, dx1, dy1 = bx + DEEP, by + DEEP, bx + bw - DEEP, by + bh - DEEP
        ix0, iy0, ix1, iy1 = bx + RING, by + RING, bx + bw - RING, by + bh - RING
        for yy in range(y0, y1):
            i0 = yy * stride
            if A[i0 + x0 * 4:i0 + x1 * 4] == B[i0 + x0 * 4:i0 + x1 * 4]:
                continue
            for xx in range(x0, x1):
                k = i0 + xx * 4
                a, b = A[k:k + 4], B[k:k + 4]
                if a == b:
                    continue
                d = max(abs(a[j] - b[j]) for j in range(3))
                in_box = bx <= xx < bx + bw and by <= yy < by + bh
                if not in_box:
                    kind = "outside"
                elif dx0 <= xx < dx1 and dy0 <= yy < dy1:
                    kind = "deep"
                elif ix0 <= xx < ix1 and iy0 <= yy < iy1:
                    kind = "interior"
                else:
                    kind = "ring"
                s = acc[kind]
                s[0] += 1
                s[1] += d
                s[2] = max(s[2], d)
    print("  [%s]" % label)
    for k in SEGS:
        c, s, m = acc[k]
        print("     %-9s 差异像素=%-9d 平均=%-6.2f 最大通道差=%d"
              % (k, c, (s / c if c else 0.0), m))
    return acc


# 重构后统一的圆角半径（= 样式里的 cornerRadius 默认值）。
# 重构前 20 块布局里有 19 块的 radius 永远停在 `Block::new` 硬编码的 28.0：
# 旧主循环 `if target_id >= 0 { 只改选中块 } else { 改全部块 }`，而 sync_selection
# 总会把 selected_id 设成最后一块，于是圆角滑杆对其余 19 块完全无效。
# 新库的半径统一由样式派生（Block::effective_radius），这是需求 F1 的直接结果，
# 属于**有意变更**，因此 many 布局的跨版本比对不能用「全 0」做判据。
R_NEW = 32.0
R_TOL = 6.0
# 圆角之外允许的最大通道差：半径变化会通过 SDF 轻微改变高斯阴影尾部，
# 实测残差 <= 2 LSB（3105 像素）。超过这个值就说明有别的东西变了。
FAR_MAX_DIFF = 2


def corner_attribution(pa, pb, rects):
    """把每一处差异归因到「圆角邻域内」或「圆角邻域外」，并给出各自的最大通道差。

    返回 (near_n, near_max, far_n, far_max)。判据：far_max 必须 <= FAR_MAX_DIFF。
    """
    A, w, h = load(pa)
    B, _, _ = load(pb)
    corners = []
    for (bx, by, bw, bh) in rects:
        for cx in (bx + R_NEW, bx + bw - R_NEW):
            for cy in (by + R_NEW, by + bh - R_NEW):
                corners.append((cx, cy))
    stride = w * 4
    near_n = near_max = far_n = far_max = 0
    for yy in range(h):
        i0 = yy * stride
        if A[i0:i0 + w * 4] == B[i0:i0 + w * 4]:
            continue
        for xx in range(w):
            k = i0 + xx * 4
            a, b = A[k:k + 4], B[k:k + 4]
            if a == b:
                continue
            d = max(abs(a[j] - b[j]) for j in range(3))
            dc = min(max(abs(xx - cx), abs(yy - cy)) for (cx, cy) in corners)
            if dc <= R_NEW + R_TOL:
                near_n += 1
                near_max = max(near_max, d)
            else:
                far_n += 1
                far_max = max(far_max, d)
    return near_n, near_max, far_n, far_max


sr = single_rect()
mr = many_rects()
# 问题二的两个光标点：都在深内部（距边 >= DEEP+60），且互相分开足够远。
# 这一问只对**当前构建**做（用 GLASS_CURSOR_AT 钉死），比的是"同一构建、只改光标"。
DEEP_A = (sr[0] + DEEP + 60, sr[1] + DEEP + 60)
DEEP_B = (sr[0] + sr[2] - DEEP - 60, sr[1] + sr[3] - DEEP - 60)

fail = []
print("=== 问题一：跨版本稳态帧（合成源，光标钉在 %s，离玻璃 > 3*mouse_radius）===" % (SAFE_CURSOR,))
r = {}
for tag, many, rects in (("single", False, [sr]), ("many", True, mr)):
    a = shot(OLD, tag + "_old", many=many, delay_ms=STEADY_MS,
             pin_cursor=SAFE_CURSOR, hold=STEADY_MS / 1000.0 + 6.0, watchdog=SAFE_CURSOR)
    b = shot(NOW, tag + "_now", many=many, delay_ms=STEADY_MS,
             pin_cursor=SAFE_CURSOR, hold=STEADY_MS / 1000.0 + 6.0, watchdog=SAFE_CURSOR)
    print("-- %s（%s）" % (tag, "20 块密集" if many else "单块"))
    r[tag] = seg_stats(a, b, rects, "old vs now @steady")
    if many:
        a_many, b_many = a, b

# many 布局比两件事：
#   (a) now vs now 的**每段最大通道差 <= 2 LSB**。不要求硬 0：模糊链(frost ping-pong)
#       与 1/16 层都是「按脏区部分刷新」，跨运行的刷新序列在桌面繁忙时会差 ±1 LSB
#       （这是重构前就有的管线特性，旧二进制同样如此；同一次运行内「增量 == 全量」
#       由 verify_dirty.py 严格证明）。single 布局 footprint 小、首帧即全覆盖，
#       因此 single 的 now-vs-now 仍然硬 0。
#   (b) old vs now 的每一处差异都必须能归因到圆角（邻域外最大通道差 <= 2），
#       因为 20 块里 19 块的半径语义是有意变更（见 R_NEW 的注释）。
print("-- many 确定性底噪（同一构建跑两次）")
c = shot(NOW, "many_now2", many=True, delay_ms=STEADY_MS,
         pin_cursor=SAFE_CURSOR, hold=STEADY_MS / 1000.0 + 6.0, watchdog=SAFE_CURSOR)
r["many_determinism"] = seg_stats(b_many, c, mr, "now vs now @steady")
near_n, near_max, far_n, far_max = corner_attribution(a_many, b_many, mr)
print("  [圆角归因] 邻域(r+%.0f)内 %d 像素/最大 %d；邻域外 %d 像素/最大 %d（阈值 %d）"
      % (R_TOL, near_n, near_max, far_n, far_max, FAR_MAX_DIFF))

print("")
print("=== 问题二：当前构建，光标在深内部两点之间移动（合成源，首帧）===")
a = shot(NOW, "cur_deep_a", pin_cursor=DEEP_A, hold=6.0)
b = shot(NOW, "cur_deep_b", pin_cursor=DEEP_B, hold=6.0)
print("-- 单块，光标 %s vs %s（都距边 >= %dpx）" % (DEEP_A, DEEP_B, DEEP))
r["cursor_deep"] = seg_stats(a, b, [sr], "cursor A vs B")

print("")
print("=== 问题三：真实桌面 composed 图（肉眼看过曝/对比度，不做逐像素比较）===")
ro = shot(OLD, "real_old", delay_ms=STEADY_MS, pin_cursor=SAFE_CURSOR,
          synth=False, hold=STEADY_MS / 1000.0 + 6.0, watchdog=SAFE_CURSOR)
rn = shot(NOW, "real_now", delay_ms=STEADY_MS, pin_cursor=SAFE_CURSOR,
          synth=False, hold=STEADY_MS / 1000.0 + 6.0, watchdog=SAFE_CURSOR)
print("  重构前:", ro)
print("  当前  :", rn)

print("")
print("=== 结论 ===")
for k, v in r.items():
    print("  %-18s deep=%-8d interior=%-8d ring=%-8d outside=%d"
          % (k, v["deep"][0], v["interior"][0], v["ring"][0], v["outside"][0]))
# single：跨版本必须逐位一致（这是"视觉效果不变"的硬证明）
for seg in SEGS:
    if r["single"][seg][0] != 0:
        fail.append("跨版本稳态帧 single 的 %s 段有 %d 像素差异（最大通道差 %d）"
                    % (seg, r["single"][seg][0], r["single"][seg][2]))
# many：同一构建两次的残差必须 <= 2 LSB（部分刷新的既有时序敏感，见上文注释）
for seg in SEGS:
    cnt, _, mx = r["many_determinism"][seg]
    if cnt and mx > 2:
        fail.append("many 布局同一构建两次跑 %s 段出现 %d 像素、最大通道差 %d (>2 LSB) —— 渲染不确定"
                    % (seg, cnt, mx))
# many：跨版本差异必须全部可归因到圆角半径的有意变更
if far_max > FAR_MAX_DIFF:
    fail.append("many 布局有 %d 像素差异落在圆角之外且通道差 %d > %d，"
                "无法用「半径 28->32 的有意变更」解释"
                % (far_n, far_max, FAR_MAX_DIFF))
if r["cursor_deep"]["deep"][0] != 0:
    fail.append("光标在深内部移动时背景发生变化：%d 像素（最大通道差 %d）"
                % (r["cursor_deep"]["deep"][0], r["cursor_deep"]["deep"][2]))
print("")
if fail:
    for f in fail:
        print("FAIL:", f)
    print("结论: FAIL 视觉取证未通过")
    sys.exit(1)
print("结论: PASS single 跨版本逐位一致；many 差异全部可归因到圆角半径的有意变更；"
      "深内部背景不随光标移动而变")
