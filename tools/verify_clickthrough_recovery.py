# -*- coding: utf-8 -*-
"""verify_clickthrough_recovery.py -- 遮挡停产后点击穿透自锁的解除取证（缺陷 #3，AC3）。

场景复刻宿主实测：不透明窗口完整盖住玻璃块 → 库进入「桌面采集停产」→ 移开窗口 →
光标进入块。此时「光标在块内 -> 解除穿透」的判定必须把 WS_EX_TRANSPARENT 摘掉；
缺陷形态下这条判定随主循环一起失去驱动源（停产期没有帧时钟，穿透态画布又收不到
鼠标消息，形成自锁），宿主实测光标进块 900ms 后 exstyle 仍带穿透位、sink 零事件。
修复形态 = 画布常驻 100ms 兜底定时器（WM_TIMER -> request_render），承诺预算 300ms。

断言链（每一步缺证据即 FAIL，禁止空转 PASS）：
  A0 初始穿透置位：光标停在块外远处，画布 exstyle 带 WS_EX_TRANSPARENT。
  A1 停产成立：盖窗后 6s 内出现「桌面采集停产」日志（场景没搭起来 = FAIL）。
  A2 遮挡移开：撤窗后 4s 内出现「遮挡已移开，恢复桌面采集」日志。
  A3 穿透解除：光标进块中心 300ms 内 exstyle 摘掉 WS_EX_TRANSPARENT（核心断言，
     记录实测毫秒数；宿主缺陷形态 >900ms 永不解除）。
  A4 反向对照（防「恒不穿透」假绿）：光标移出块到远处，1000ms 内穿透位必须重新置上。
  A5 交互真实恢复：物理按下块右半（DragMove 区）拖动，sink rect 心跳随手移动。
  A6 引擎全程存活：结束时进程在跑且 [metrics] 在追加。

已知环境依赖：块被盖住的判定（B1）要求遮挡者不透明、非 layered、z 序在画布之上；
本机画布是 WS_EX_TOPMOST，所以探针窗口必须抬进 topmost 带（与 verify_occlusion 同一
手法）。探针窗口带 WS_CAPTION 以绕开独占全屏判据（那是 A 门的刺激，不是这里的）。
"""
import ctypes
import os
import re
import subprocess
import threading
import time
from ctypes import c_void_p, c_int, c_uint, c_long, Structure, WINFUNCTYPE

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")

user32 = ctypes.windll.user32
gdi32 = ctypes.windll.gdi32
k32 = ctypes.windll.kernel32
try:
    user32.SetProcessDpiAwarenessContext(c_void_p(-4))
except Exception:
    user32.SetProcessDPIAware()

# --- Win32 常量 ---
WM_PAINT, WM_DESTROY = 0x000F, 0x0002
WS_POPUP = 0x80000000
WS_VISIBLE = 0x10000000
WS_CAPTION = 0x00C00000
GWL_EXSTYLE = -20
WS_EX_TRANSPARENT = 0x00000020
HWND_TOPMOST = c_void_p(-1)
SWP_NOSIZE, SWP_NOMOVE, SWP_NOACTIVATE, SWP_SHOWWINDOW = 1, 2, 0x10, 0x40
SM_CXSCREEN, SM_CYSCREEN = 0, 1
LEFTDOWN, LEFTUP = 0x0002, 0x0004
PM_REMOVE = 0x0001
CANVAS_CLASS = "LiquidVerreCanvas"


class RECT(Structure):
    _fields_ = [("left", c_long), ("top", c_long), ("right", c_long), ("bottom", c_long)]


class MSG(Structure):
    _fields_ = [("hwnd", c_void_p), ("message", c_uint), ("wParam", c_void_p),
                ("lParam", c_void_p), ("time", c_uint), ("pt_x", c_long), ("pt_y", c_long)]


class WNDCLASSW(Structure):
    _fields_ = [("style", c_uint), ("lpfnWndProc", c_void_p), ("cbClsExtra", c_int),
                ("cbWndExtra", c_int), ("hInstance", c_void_p), ("hIcon", c_void_p),
                ("hCursor", c_void_p), ("hbrBackground", c_void_p),
                ("lpszMenuName", ctypes.c_wchar_p), ("lpszClassName", ctypes.c_wchar_p)]


# ctypes 原型必须显式声明（64 位下默认 c_int 会截断 HWND —— verify_occlusion 前科）。
user32.CreateWindowExW.restype = c_void_p
user32.CreateWindowExW.argtypes = [c_uint, c_void_p, ctypes.c_wchar_p, c_uint, c_int, c_int,
                                   c_int, c_int, c_void_p, c_void_p, c_void_p, c_void_p]
user32.DefWindowProcW.restype = ctypes.c_ssize_t
user32.DefWindowProcW.argtypes = [c_void_p, c_uint, c_void_p, c_void_p]
user32.RegisterClassW.restype = c_uint
user32.RegisterClassW.argtypes = [ctypes.POINTER(WNDCLASSW)]
user32.BeginPaint.restype = c_void_p
user32.BeginPaint.argtypes = [c_void_p, c_void_p]
user32.EndPaint.argtypes = [c_void_p, c_void_p]
user32.GetClientRect.argtypes = [c_void_p, ctypes.POINTER(RECT)]
user32.SetWindowPos.restype = c_int
user32.SetWindowPos.argtypes = [c_void_p, c_void_p, c_int, c_int, c_int, c_int, c_uint]
user32.DestroyWindow.argtypes = [c_void_p]
user32.UpdateWindow.argtypes = [c_void_p]
user32.PeekMessageW.argtypes = [ctypes.POINTER(MSG), c_void_p, c_uint, c_uint, c_uint]
user32.TranslateMessage.argtypes = [ctypes.POINTER(MSG)]
user32.DispatchMessageW.argtypes = [ctypes.POINTER(MSG)]
user32.FillRect.argtypes = [c_void_p, ctypes.POINTER(RECT), c_void_p]
user32.InvalidateRect.argtypes = [c_void_p, c_void_p, c_int]
user32.FindWindowExW.restype = c_void_p
user32.FindWindowExW.argtypes = [c_void_p, c_void_p, ctypes.c_wchar_p, ctypes.c_wchar_p]
user32.GetWindowThreadProcessId.restype = c_uint
user32.GetWindowThreadProcessId.argtypes = [c_void_p, ctypes.POINTER(c_uint)]
user32.GetWindowLongW.restype = c_uint
user32.GetWindowLongW.argtypes = [c_void_p, c_int]
gdi32.CreateSolidBrush.restype = c_void_p
gdi32.CreateSolidBrush.argtypes = [c_uint]
k32.GetModuleHandleW.restype = c_void_p
k32.GetModuleHandleW.argtypes = [ctypes.c_wchar_p]

WNDPROC = WINFUNCTYPE(ctypes.c_ssize_t, c_void_p, c_uint, c_void_p, c_void_p)
_BRUSH = [None]
_CLASS_ATOM = [None]
_COVER = [None]


def _wndproc(hwnd, msg, wp, lp):
    if msg == WM_PAINT:
        ps = (ctypes.c_byte * 136)()
        hdc = user32.BeginPaint(c_void_p(hwnd), ctypes.byref(ps))
        if hdc:
            rc = RECT()
            user32.GetClientRect(c_void_p(hwnd), ctypes.byref(rc))
            user32.FillRect(c_void_p(hdc), ctypes.byref(rc), _BRUSH[0])
        user32.EndPaint(c_void_p(hwnd), ctypes.byref(ps))
        return 0
    if msg == WM_DESTROY:
        return 0
    return user32.DefWindowProcW(c_void_p(hwnd), msg, c_void_p(wp), c_void_p(lp))


_WNDPROC_REF = WNDPROC(_wndproc)  # 回调必须持有引用，GC 后系统调进已释放内存


def _hinst():
    return k32.GetModuleHandleW(None)


def make_cover():
    """B1 正向刺激：不透明、铺满主屏、topmost、带 caption（绕开 A 门判据）。"""
    if not _CLASS_ATOM[0]:
        _BRUSH[0] = gdi32.CreateSolidBrush(0x00404040)
        wc = WNDCLASSW()
        wc.lpfnWndProc = ctypes.cast(_WNDPROC_REF, c_void_p).value
        wc.hInstance = _hinst()
        wc.hbrBackground = _BRUSH[0]
        wc.lpszClassName = "VerreCTRecoveryProbe"
        _CLASS_ATOM[0] = user32.RegisterClassW(ctypes.byref(wc))
        if not _CLASS_ATOM[0]:
            raise RuntimeError("RegisterClassW failed")
    w = user32.GetSystemMetrics(SM_CXSCREEN)
    h = user32.GetSystemMetrics(SM_CYSCREEN)
    hwnd = user32.CreateWindowExW(0, _CLASS_ATOM[0], "cover",
                                  WS_POPUP | WS_CAPTION | WS_VISIBLE,
                                  0, 0, w, h, None, None, _hinst(), None)
    if not hwnd:
        raise RuntimeError("CreateWindowExW(cover) failed")
    user32.SetWindowPos(c_void_p(hwnd), HWND_TOPMOST, 0, 0, 0, 0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW)
    user32.UpdateWindow(c_void_p(hwnd))
    _COVER[0] = hwnd
    return hwnd


def destroy_cover():
    if _COVER[0]:
        user32.DestroyWindow(c_void_p(_COVER[0]))
        _COVER[0] = None


def pump(seconds):
    """给探针窗口跑消息泵（不泵就不画 -> 不透明判据失真）。"""
    end = time.perf_counter() + seconds
    m = MSG()
    while time.perf_counter() < end:
        while user32.PeekMessageW(ctypes.byref(m), None, 0, 0, PM_REMOVE):
            user32.TranslateMessage(ctypes.byref(m))
            user32.DispatchMessageW(ctypes.byref(m))
        time.sleep(0.02)


RESULTS = []


def check(name, ok, detail):
    print(("  PASS: " if ok else "  FAIL: ") + name + " -- " + detail)
    RESULTS.append(bool(ok))
    return ok


STATE_RX = re.compile(
    r"sink_demo_state id=\d+ rect=\[(-?[\d.]+),\s*(-?[\d.]+),\s*(-?[\d.]+),\s*(-?[\d.]+)\] scale=([\d.]+)")


class Session:
    def __init__(self):
        env = os.environ.copy()
        env["GLASS_DEFAULT_BLOCK"] = "1"
        env["GLASS_MONITORS"] = "1"
        env["GLASS_METRICS"] = "1"
        env["GLASS_ADAPTIVE"] = "0"
        env["GLASS_FS_HIDE"] = "0"
        env["GLASS_INPUT_SINK_DEMO"] = "1"
        env["GLASS_OCCLUSION"] = "1"  # B1 场景必须先成立
        for k in ("GLASS_MANY", "GLASS_FAR_GRID", "GLASS_SPREAD", "GLASS_SIM_BACKDROP",
                  "GLASS_SIM_BUSY", "GLASS_CURSOR_AT", "GLASS_INTERNAL_SHOT",
                  "GLASS_INTERNAL_SHOT_AT_MS", "GLASS_VERIFY_VALIDITY"):
            env.pop(k, None)
        self.proc = subprocess.Popen(
            [EXE, "--no-browser", "--no-demo-pair"], env=env, cwd=ROOT,
            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
            text=True, encoding="utf-8", errors="replace")
        self.lines = []
        self.lock = threading.Lock()
        threading.Thread(target=self._pump, daemon=True).start()

    def _pump(self):
        try:
            for line in self.proc.stderr:
                with self.lock:
                    self.lines.append(line.rstrip())
        except Exception:
            pass

    def snapshot(self):
        with self.lock:
            return list(self.lines)

    def last_state(self):
        for line in reversed(self.snapshot()):
            m = STATE_RX.search(line)
            if m:
                return tuple(float(g) for g in m.groups())
        return None

    def wait_mark(self, needle, budget, mark_len=None):
        """等待某段日志文本出现（needle 按子串匹配，与 verify_occlusion MARKS 同口径）。
        返回出现时刻（秒）或 None。期间必须 pump：覆盖窗还活着，不泵就白屏。"""
        t0 = time.time()
        while time.time() - t0 < budget:
            if mark_len is None:
                if any(needle in l for l in self.snapshot()):
                    return time.time() - t0
            else:
                if len(self.snapshot()) > mark_len and any(needle in l for l in self.snapshot()):
                    return time.time() - t0
            pump(0.15)
        return None

    def alive(self):
        return self.proc.poll() is None

    def close(self):
        destroy_cover()
        self.proc.terminate()
        try:
            self.proc.wait(timeout=6)
        except subprocess.TimeoutExpired:
            self.proc.kill()


def find_canvas_all(pid=None):
    """按类名链式枚举画布；pid 给定时只收该进程的窗口。

    必须按 pid 过滤：同机可能残留上一轮取证进程、或本进程 truncate 泄漏的孤儿画布
    （恒穿透、永不变化）—— 打中错窗口会把活测成死（本脚本第一版就栽在这里）。
    """
    wins = []
    h = 0
    while True:
        h = user32.FindWindowExW(0, h, CANVAS_CLASS, None)
        if not h:
            break
        if pid is not None:
            got = c_uint()
            user32.GetWindowThreadProcessId(c_void_p(h), ctypes.byref(got))
            if got.value != pid:
                continue
        wins.append(int(h))
    return wins


def ex_transparent(hwnd):
    return (user32.GetWindowLongW(c_void_p(hwnd), GWL_EXSTYLE) & WS_EX_TRANSPARENT) != 0


def park_cursor():
    sh = user32.GetSystemMetrics(SM_CYSCREEN)
    user32.SetCursorPos(8, sh - 8)  # 左上角块外远处（家族同款停放点）


def drag(x0, y0, dx, dy, steps=5):
    user32.SetCursorPos(int(x0), int(y0))
    time.sleep(0.12)
    user32.mouse_event(LEFTDOWN, 0, 0, 0, 0)
    time.sleep(0.08)
    for i in range(1, steps + 1):
        user32.SetCursorPos(int(x0 + dx * i / steps), int(y0 + dy * i / steps))
        time.sleep(0.09)
    user32.mouse_event(LEFTUP, 0, 0, 0, 0)
    time.sleep(0.06)


def summarize():
    n_fail = RESULTS.count(False)
    print("\n==============================================")
    if n_fail == 0:
        print("总体结论: PASS 停产-移开-进块链路在预算内自动解除穿透，交互真实恢复")
        return 0
    print("总体结论: FAIL %d 条断言未通过" % n_fail)
    return 1


def main():
    print("EXE = %s" % EXE)
    if not os.path.isfile(EXE):
        print("FAIL: 被测 EXE 不存在（先 cargo build --release）")
        return 1
    sess = Session()
    deadline = time.time() + 15.0
    while time.time() < deadline and sess.last_state() is None:
        time.sleep(0.2)
    g0 = sess.last_state()
    if not check("启动成功且 sink rect 心跳可见", g0 is not None, str(g0)):
        sess.close()
        return summarize()
    x, y, w, h, scale = g0
    # sink_demo_state 的 rect 是虚拟屏物理像素（Block 内部单位直传快照）。
    px, py, pw, ph = x, y, w, h
    canvases = find_canvas_all(sess.proc.pid)
    if not check("枚举到本进程画布", len(canvases) >= 1, "canvases=%s pid=%d" % (canvases, sess.proc.pid)):
        sess.close()
        return summarize()
    print("  info: 画布枚举到 %d 块（用解除事件动态识别活动画布）" % len(canvases))
    # ---- A0 初始全穿透：解除发生前没有任何画布提前摘穿透（场景没搭好 = FAIL）----
    park_cursor()
    t0 = time.time()
    ok0 = False
    while time.time() - t0 < 1.5:
        if all(ex_transparent(hw) for hw in canvases):
            ok0 = True
            break
        time.sleep(0.05)
    if not check("A0 光标在块外时全部画布穿透置位", ok0, "canvases=%d" % len(canvases)):
        sess.close()
        return summarize()
    # ---- A1 盖住 -> 停产 ----
    print("\n--- 盖不透明窗口，等「桌面采集停产」 ---")
    make_cover()
    t_paused = sess.wait_mark("桌面采集停产", 6.0)
    if not check("A1 遮挡停产成立", t_paused is not None, "t=%.2fs" % (t_paused if t_paused is not None else -1)):
        sess.close()
        return summarize()
    # 停产态多停留 1s：把循环推到「无帧时钟」的真实形态
    pump(1.0)
    # ---- A2 移开 -> 恢复 ----
    print("--- 撤掉遮挡，等「遮挡已移开」 ---")
    destroy_cover()
    t_off = sess.wait_mark("遮挡已移开，恢复桌面采集", 4.0)
    pump(0.6)
    if not check("A2 遮挡解除被检出", t_off is not None, "t=%.2fs" % (t_off if t_off is not None else -1)):
        sess.close()
        return summarize()
    # ---- A3 光标进块：300ms 内有画布解除穿透（核心断言 + 活动画布识别）----
    print("--- SetCursorPos 进块中心，轮询各画布 exstyle（预算 300ms）---")
    cx, cy = px + pw * 0.55, py + ph * 0.5
    user32.SetCursorPos(int(cx), int(cy))
    active, t_clear, t0 = None, None, time.time()
    while time.time() - t0 < 1.0:
        for hw in canvases:
            if not ex_transparent(hw):
                active, t_clear = hw, time.time() - t0
                break
        if active:
            break
        time.sleep(0.02)
    ms_txt = ("%.0f ms" % (t_clear * 1000)) if t_clear is not None else "1000ms 内未解除"
    ok3 = active is not None and t_clear is not None and t_clear <= 0.30
    check("A3 光标进块 <=300ms 摘掉 WS_EX_TRANSPARENT", ok3, "实测 %s active=%s" % (ms_txt, active))
    if active is None:
        check("A3b 无法识别活动画布（全部画布恒穿透 = 缺陷 #3 形态）", False, "后续断言跳过")
        sess.close()
        return summarize()
    # ---- A4 反向对照（活动画布）：光标回块外，穿透必须回来 ----
    park_cursor()
    t_back, t0 = None, time.time()
    while time.time() - t0 < 1.5:
        if ex_transparent(active):
            t_back = time.time() - t0
            break
        time.sleep(0.03)
    tb = ("%.0f ms" % (t_back * 1000)) if t_back is not None else "1500ms 未恢复穿透"
    check("A4 光标离开块后活动画布重新置上穿透位（反向对照）", t_back is not None, "t=" + tb)
    # ---- A5 物理拖拽：DragMove 半区，块必须随手走 ----
    print("--- 物理按下拖动块右半（DragMove 区）---")
    gx, gy = px + pw * 0.75, py + ph * 0.5
    dx_phys = min(160.0, pw * 0.3)
    drag(gx, gy, dx_phys, 0)
    pump(0.6)
    g1 = sess.last_state()
    moved = (g1[0] - x) if g1 else -1
    check("A5 停产后交互真实恢复（块被拖走）", g1 is not None and moved >= dx_phys * 0.5,
          "dx=%.1f 期望>=%.1f" % (moved, dx_phys * 0.5))
    # ---- A6 引擎存活 + metrics 追加 ----
    # [metrics] 是 5 秒墙钟窗口结算：前面各段合计约 6 s，必须再等出一个完整窗口
    # 才允许断言（第一版在此误报 metrics=0，实为「数早了」而非缺陷）。
    time.sleep(6.0)
    n_metrics = sum(1 for l in sess.snapshot() if "[metrics]" in l)
    check("A6 引擎全程存活且 metrics 有结算（>=1 行）", sess.alive() and n_metrics >= 1,
          "alive=%s metrics=%d" % (sess.alive(), n_metrics))
    sess.close()
    return summarize()


if __name__ == "__main__":
    import sys
    sys.exit(main())
