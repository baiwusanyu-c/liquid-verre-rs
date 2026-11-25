# -*- coding: utf-8 -*-
"""verify_desktop_foreground.py -- 桌面壳（Progman / WorkerW）不得触发独占全屏停产（0.5.4）。

背景（宿主定位报告 2026-11，Win10 + Wallpaper Engine 实测）：WE 把壁纸挂在 `WorkerW`
兄弟窗口里；Win10 用户点一下桌面空白处，前台窗口就是 WorkerW。它的形状
（WS_POPUP、无 caption/thickframe、铺满屏）与 Progman 同形，**三条形状判据全中** ——
0.4.2 修 Progman 误报时只挡了 GetShellWindow 返回的那一个 HWND，WorkerW 漏网，
于是「分类框整体消失、点任务栏又回来」。本脚本在**没有 WE 的机器**上复现同形刺激：
在本进程注册类名 WorkerW / Progman 的铺满屏窗口（类名是进程 atom，不需要 WE 在场），
顶到前台，验库不再误停产。

阶段（GLASS_OCCLUSION=0 全程：dda 归零只可能来自 A 的停产门，归因无歧义）：

  S0 baseline          只有角落闪动小窗（制造真实脏区） -> dda>1、无 fs_on、画布可见
  S1 对照组(真实形态)   类名 VerreFakeFS2 全屏弹窗+前台  -> **必须** fs_on、dda≈0、画布被隐藏
                       （护栏：类名排除不许误伤真全屏应用，否则 R-16 报废）
  S2 恢复              撤掉对照窗口                     -> fs_off、dda 回升、画布重新可见
  S3 假 WorkerW        类名 WorkerW 同形 + 前台         -> **不许** fs_on、dda>1、画布可见
  S4 假 Progman 兜底    类名 Progman 同形 + 前台         -> **不许** fs_on、dda>1、画布可见
                       （GetShellWindow 返回 explorer 的 HWND，本进程假 Progman 身份
                        比较必输 —— 这一阶段专门测「类名兜底」那一半修复）

判别力：修复前的实现会把 S3/S4 测红（fs_on 出现、dda 归零、画布隐藏）；
S1 保证修复没把真判据废掉。前台被桌面抢走（实测存在）计入 drift：S1 在漂移时
放宽 dda 门槛（同 verify_occlusion 的 fg_gate 思路），S3/S4 的 require_fg 保证
「前台从没拿到 = 没测」直接 FAIL。

⚠ 本脚本首轮运行还把一条**相邻缺陷**打了出来（S4 被副屏抢前台的
XamlExplorerHostIslandWindow 误判为独占全屏 → 主屏画布被隐藏）：
旧实现的铺满判据量的是「前台窗口自己所在的屏」而不是「画布所在的屏」，
副屏全屏会误伤主屏玻璃。现已按屏收窄（display::fullscreen_app_on 收
canvas 的 HMONITOR）。所以脚本要求被测环境里**副屏行为不越界**：这也是
常驻断言的一部分，写回去就红。

已知环境依赖：若系统把 Progman/WorkerW 注册成了**全局类**（CS_GLOBALCLASS），
RegisterClassW 会失败 ⇒ 直接用类名字符串建窗即可命中那个全局类；两条路径类名一致，
判据不变（库读的是 GetClassNameW 的返回）。
"""
import ctypes
import io
import os
import re
import subprocess
import sys
import time
from ctypes import c_void_p, c_int, c_uint, c_long, c_size_t, Structure, WINFUNCTYPE

# 控制台 GBK 时中文可写而特殊符号会 UnicodeEncodeError：统一转 UTF-8 兜底。
if hasattr(sys.stdout, "reconfigure"):
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

try:
    ctypes.windll.user32.SetProcessDpiAwarenessContext(c_void_p(-4))
except Exception:
    try:
        ctypes.windll.user32.SetProcessDPIAware()
    except Exception:
        pass

u = ctypes.windll.user32
g = ctypes.windll.gdi32
k32 = ctypes.windll.kernel32

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")
LOG = os.path.join(ROOT, "tools", "desktop_fg.log")
CANVAS_CLASS = "LiquidVerreCanvas"

# --- Win32 常量与原型（64 位 HWND 截断前科：所有返回句柄的 API 显式 restype）---
WM_DESTROY = 0x0002
WM_PAINT = 0x000F
WS_POPUP = 0x80000000
WS_VISIBLE = 0x10000000
WS_EX_TOPMOST = 0x00000008
HWND_TOPMOST = c_void_p(-1)
SWP_NOSIZE = 0x0001
SWP_NOMOVE = 0x0002
SWP_NOACTIVATE = 0x0010
SWP_SHOWWINDOW = 0x0040
SM_CXSCREEN = 0
SM_CYSCREEN = 1
VK_MENU = 0xA4
KEYEVENTF_KEYUP = 0x0002
PM_REMOVE = 0x0001

SETTLE = float(os.environ.get("VD_SETTLE", "5.4"))
MEASURE = float(os.environ.get("VD_MEASURE", "5.4"))
STARTUP = float(os.environ.get("VD_STARTUP", "6.0"))


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


u.CreateWindowExW.restype = c_void_p
# 类名参数走 c_wchar_p（字符串查找与 atom 等价，见模块文档的环境依赖）。
u.CreateWindowExW.argtypes = [c_uint, ctypes.c_wchar_p, ctypes.c_wchar_p, c_uint,
                              c_int, c_int, c_int, c_int,
                              c_void_p, c_void_p, c_void_p, c_void_p]
u.DefWindowProcW.restype = ctypes.c_ssize_t
u.DefWindowProcW.argtypes = [c_void_p, c_uint, c_void_p, c_void_p]
u.RegisterClassW.restype = c_uint
u.RegisterClassW.argtypes = [ctypes.POINTER(WNDCLASSW)]
u.SetWindowPos.restype = c_int
u.SetWindowPos.argtypes = [c_void_p, c_void_p, c_int, c_int, c_int, c_int, c_uint]
u.DestroyWindow.argtypes = [c_void_p]
u.UpdateWindow.argtypes = [c_void_p]
u.ShowWindow.restype = c_int
u.ShowWindow.argtypes = [c_void_p, c_int]
u.GetForegroundWindow.restype = c_void_p
u.SetForegroundWindow.restype = c_int
u.SetForegroundWindow.argtypes = [c_void_p]
u.BringWindowToTop.argtypes = [c_void_p]
u.GetWindowThreadProcessId.restype = c_uint
u.GetWindowThreadProcessId.argtypes = [c_void_p, c_void_p]
u.AttachThreadInput.restype = c_int
u.AttachThreadInput.argtypes = [c_uint, c_uint, c_int]
u.PeekMessageW.argtypes = [ctypes.POINTER(MSG), c_void_p, c_uint, c_uint, c_uint]
u.TranslateMessage.argtypes = [ctypes.POINTER(MSG)]
u.DispatchMessageW.argtypes = [ctypes.POINTER(MSG)]
u.FillRect.argtypes = [c_void_p, ctypes.POINTER(RECT), c_void_p]
u.InvalidateRect.argtypes = [c_void_p, ctypes.POINTER(RECT), c_int]
u.BeginPaint.restype = c_void_p
u.BeginPaint.argtypes = [c_void_p, c_void_p]
u.EndPaint.argtypes = [c_void_p, c_void_p]
u.GetClientRect.argtypes = [c_void_p, ctypes.POINTER(RECT)]
u.IsWindowVisible.restype = c_int
u.IsWindowVisible.argtypes = [c_void_p]
u.FindWindowExW.restype = c_void_p
u.FindWindowExW.argtypes = [c_void_p, c_void_p, ctypes.c_wchar_p, ctypes.c_wchar_p]
u.GetSystemMetrics.restype = c_int
u.GetSystemMetrics.argtypes = [c_int]
u.SetCursorPos.restype = c_int
u.SetCursorPos.argtypes = [c_int, c_int]
u.keybd_event.argtypes = [c_uint, c_uint, c_uint, c_size_t]
g.CreateSolidBrush.restype = c_void_p
g.CreateSolidBrush.argtypes = [c_uint]
k32.GetModuleHandleW.restype = c_void_p
k32.GetModuleHandleW.argtypes = [ctypes.c_wchar_p]
k32.GetCurrentThreadId.restype = c_uint

WNDPROC = WINFUNCTYPE(ctypes.c_ssize_t, c_void_p, c_uint, c_void_p, c_void_p)
_BRUSH = g.CreateSolidBrush(0x00404040)     # COLORREF 0x00BBGGRR：深灰
_BRUSH2 = g.CreateSolidBrush(0x00414141)    # 只差 1 LSB：肉眼无感但每帧真实脏区
_BLINK = [None]        # 当前全屏覆盖窗（只闪正中 96x96，整窗闪会把机器打成劣化窗口）
_TICKER = [None]       # 角落常驻小闪窗（保证任何阶段桌面都有脏区，dda 有牙）
_BLINK_ON = [0]
_FG_HOLD = [None]
_FG_DRIFT = [0]
_FG_NEXT = [0.0]
_COVER = [None]
_HINST = [None]
_REGISTERED = set()


def _wndproc(hwnd, msg, wp, lp):
    if msg == WM_PAINT:
        ps = (ctypes.c_byte * 136)()
        hdc = u.BeginPaint(c_void_p(hwnd), ctypes.byref(ps))
        if hdc:
            rc = RECT()
            u.GetClientRect(c_void_p(hwnd), ctypes.byref(rc))
            u.FillRect(c_void_p(hdc), ctypes.byref(rc), _BRUSH2 if _BLINK_ON[0] else _BRUSH)
        u.EndPaint(c_void_p(hwnd), ctypes.byref(ps))
        return 0
    if msg == WM_DESTROY:
        return 0
    return u.DefWindowProcW(c_void_p(hwnd), msg, c_void_p(wp), c_void_p(lp))


# 回调对象必须持引用，否则 GC 后 Windows 调进已释放内存 -> 崩溃（occlusion 同款注释）。
_WNDPROC_REF = WNDPROC(_wndproc)


def hinstance():
    if _HINST[0] is None:
        _HINST[0] = k32.GetModuleHandleW(None)
    return _HINST[0]


def ensure_class(name):
    """在本进程注册指定类名的窗口类。返回是否注册成功。

    失败也不致命：可能系统已有同名**全局**类（某些 Progman 变体），
    CreateWindowExW 用字符串类名照样能命中；注册成功与否都不改变
    「最终窗口的 GetClassNameW == name」这一测试前提。
    """
    if name in _REGISTERED:
        return True
    wc = WNDCLASSW()
    wc.style = 0
    wc.lpfnWndProc = ctypes.cast(_WNDPROC_REF, c_void_p).value
    wc.hInstance = hinstance()
    wc.hbrBackground = _BRUSH
    wc.lpszClassName = name
    if u.RegisterClassW(ctypes.byref(wc)):
        _REGISTERED.add(name)
        return True
    _REGISTERED.add(name)  # 已存在：走字符串类名路径
    return False


def screen_size():
    return u.GetSystemMetrics(SM_CXSCREEN), u.GetSystemMetrics(SM_CYSCREEN)


def _popup(class_name, title, topmost=True):
    ensure_class(class_name)
    w, h = screen_size()
    hwnd = u.CreateWindowExW(0, class_name, title, WS_POPUP | WS_VISIBLE,
                             0, 0, w, h, None, None, hinstance(), None)
    if not hwnd:
        raise RuntimeError("CreateWindowExW(%s) failed, gle=%s"
                           % (class_name, ctypes.GetLastError()))
    if topmost:
        u.SetWindowPos(c_void_p(hwnd), HWND_TOPMOST, 0, 0, 0, 0,
                       SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW)
    u.UpdateWindow(c_void_p(hwnd))
    return hwnd


def make_ticker():
    """角落 96x96 常驻闪窗：带 WS_CAPTION（永远不满足 A 的形状判据），只负责脏区。"""
    ensure_class("VerreTicker")
    w, h = screen_size()
    hwnd = u.CreateWindowExW(WS_EX_TOPMOST, "VerreTicker", "verre ticker",
                             WS_POPUP | WS_VISIBLE | 0x00C00000,
                             max(0, w - 150), max(0, h - 150), 96, 96,
                             None, None, hinstance(), None)
    if not hwnd:
        raise RuntimeError("ticker create failed")
    _TICKER[0] = hwnd
    return hwnd


def destroy(hwnd):
    if hwnd:
        if _BLINK[0] == hwnd:
            _BLINK[0] = None
        if _FG_HOLD[0] == hwnd:
            _FG_HOLD[0] = None
        u.DestroyWindow(c_void_p(hwnd))


def force_foreground(hwnd):
    """拿到前台并**自证**成功（Alt + AttachThreadInput 组合，同 verify_occlusion）。"""
    got = False
    for _ in range(6):
        u.keybd_event(VK_MENU, 0, 0, 0)
        u.keybd_event(VK_MENU, 0, KEYEVENTF_KEYUP, 0)
        fg = u.GetForegroundWindow()
        fgtid = u.GetWindowThreadProcessId(c_void_p(fg), None) if fg else 0
        curtid = k32.GetCurrentThreadId()
        attached = bool(fgtid) and fgtid != curtid and bool(
            u.AttachThreadInput(fgtid, curtid, True))
        u.SetForegroundWindow(c_void_p(hwnd))
        u.BringWindowToTop(c_void_p(hwnd))
        end = time.perf_counter() + 0.5
        while time.perf_counter() < end:
            pump(0.05)
            if u.GetForegroundWindow() == hwnd:
                got = True
                break
        if attached:
            u.AttachThreadInput(fgtid, curtid, False)
        if got:
            break
        pump(0.3)
    if got:
        _FG_HOLD[0] = hwnd
    return got


def pump(seconds):
    end = time.perf_counter() + seconds
    next_blink = time.perf_counter()
    while time.perf_counter() < end:
        m = MSG()
        while u.PeekMessageW(ctypes.byref(m), None, 0, 0, PM_REMOVE):
            u.TranslateMessage(ctypes.byref(m))
            u.DispatchMessageW(ctypes.byref(m))
        now = time.perf_counter()
        h_fg = _FG_HOLD[0]
        if h_fg and now >= _FG_NEXT[0]:
            _FG_NEXT[0] = now + 0.2
            if u.GetForegroundWindow() != h_fg:
                _FG_DRIFT[0] += 1
                u.SetForegroundWindow(c_void_p(h_fg))
                u.BringWindowToTop(c_void_p(h_fg))
        if now >= next_blink:
            next_blink = now + 0.1
            _BLINK_ON[0] ^= 1
            if _BLINK[0]:
                # 覆盖窗只闪正中 96x96（整窗闪 = 每秒 6500 万像素 GDI 填充，劣化前科）。
                rc = RECT()
                w, hh = screen_size()
                rc.left, rc.top = w // 2 - 48, hh // 2 - 48
                rc.right, rc.bottom = w // 2 + 48, hh // 2 + 48
                u.InvalidateRect(c_void_p(_BLINK[0]), ctypes.byref(rc), 0)
                u.UpdateWindow(c_void_p(_BLINK[0]))
            if _TICKER[0]:
                # 小窗整窗闪（96x96，成本可忽略）。
                u.InvalidateRect(c_void_p(_TICKER[0]), None, 0)
                u.UpdateWindow(c_void_p(_TICKER[0]))
        time.sleep(0.02)


# --- metrics / 日志 ---------------------------------------------------------
NUM = {
    "dda_acq": re.compile(rb"dda_acq=([0-9.]+)/s"),
    "fps": re.compile(rb"fps=([0-9.]+)"),
}
MARKS = {
    "fs_on": "检测到独占全屏应用，暂时隐藏玻璃".encode("utf-8"),
    "fs_off": "独占全屏应用已退出，恢复玻璃".encode("utf-8"),
}


def read_new(path, offset):
    try:
        with io.open(path, "rb") as fh:
            fh.seek(offset)
            data = fh.read()
            offset = fh.tell()
    except OSError:
        return offset, []
    return offset, [ln for ln in data.split(b"\n") if ln.strip()]


def med(vals):
    if not vals:
        return None
    v = sorted(vals)
    return v[len(v) // 2]


def canvases(pid):
    """按 pid 过滤枚举自家画布（孤儿画布前科：truncate 泄漏，不按 pid 过滤会打中错窗口）。"""
    out = []
    h = None
    while True:
        h = u.FindWindowExW(None, c_void_p(h) if h else None, CANVAS_CLASS, None)
        if not h:
            break
        got = ctypes.c_uint()
        u.GetWindowThreadProcessId(c_void_p(h), ctypes.byref(got))
        if got.value == pid:
            out.append(int(h))
    return out


def canvas_any_visible(pid):
    return any(u.IsWindowVisible(c_void_p(h)) for h in canvases(pid))


class Probe(object):
    def __init__(self, extra_env=None):
        env = dict(os.environ)
        env["GLASS_DEFAULT_BLOCK"] = "1"
        env["GLASS_MONITORS"] = "1"
        env["GLASS_METRICS"] = "1"
        env["GLASS_ADAPTIVE"] = "0"
        # B1 全程关：dda 归零只可能来自 A 的停产门，S1/S3/S4 的归因都没有第二条路。
        env["GLASS_OCCLUSION"] = "0"
        for k in ("GLASS_FS_HIDE", "GLASS_BACKEND", "GLASS_BLACK_PROBE", "GLASS_MANY",
                  "GLASS_FAR_GRID", "GLASS_SPREAD", "GLASS_SIM_BACKDROP", "GLASS_SIM_BUSY",
                  "GLASS_CURSOR_AT", "GLASS_INTERNAL_SHOT", "GLASS_SYNTH_CAP"):
            env.pop(k, None)
        if extra_env:
            env.update(extra_env)
        if os.path.exists(LOG):
            os.remove(LOG)
        self.fh = io.open(LOG, "wb")
        self.p = subprocess.Popen([EXE, "--no-browser"], env=env, cwd=ROOT,
                                  stdout=subprocess.DEVNULL, stderr=self.fh)
        self.off = 0
        self.raw = []

    def drain(self):
        self.off, lines = read_new(LOG, self.off)
        self.raw.extend(lines)
        rows = []
        for line in lines:
            if b"[metrics]" not in line:
                continue
            s = {}
            for k, rx in NUM.items():
                m = rx.search(line)
                if m:
                    s[k] = float(m.group(1))
            if s:
                rows.append(s)
        return rows

    def stop(self):
        try:
            self.p.terminate()
            self.p.wait(timeout=10)
        except Exception:
            try:
                self.p.kill()
            except Exception:
                pass
        try:
            self.fh.close()
        except Exception:
            pass


def phase(probe, label, cover_factory, expect_fn, marks=(), require_fg=False,
          keep_raw=False):
    """一个阶段：撤旧覆盖 -> 建新覆盖并夺前台 -> 沉降 -> 采到 metrics 为止 -> 判定。

    状态翻转只由本函数的「建/撤覆盖」驱动（teardown 会把翻转沿日志落进下一段，
    被 del raw 清掉 => 假 FAIL，verify_occlusion 前科）。
    expect_fn(dda, fps, canvas_visible, drift) -> (bool, why)
    """
    if not keep_raw:
        del probe.raw[:]
    drift0 = _FG_DRIFT[0]
    destroy(_COVER[0])
    _COVER[0] = None
    fg_ok = None
    if cover_factory:
        _COVER[0] = cover_factory()
        _BLINK[0] = _COVER[0]
        fg_ok = force_foreground(_COVER[0])
    pump(SETTLE)
    probe.drain()
    rows = []
    deadline = time.perf_counter() + MEASURE * 3.0
    while not rows and time.perf_counter() < deadline:
        pump(2.0)
        rows.extend(probe.drain())
    dda = med([r["dda_acq"] for r in rows if "dda_acq" in r])
    fps = med([r["fps"] for r in rows if "fps" in r])
    vis = canvas_any_visible(probe.p.pid)
    drift = _FG_DRIFT[0] - drift0
    problems = []
    if require_fg and fg_ok is False:
        problems.append("SetForegroundWindow 没拿到前台，本阶段等于没测")
    ok, why = expect_fn(dda, fps, vis, drift)
    if not ok:
        problems.append(why)
    for key, required in marks:
        found = any(MARKS[key] in ln for ln in probe.raw)
        if found != required:
            problems.append("日志%s「%s」翻转沿" %
                            ("缺少" if required else "出现了不该有的", key))
    print("  %-30s n=%d dda=%s fps=%s 画布可见=%s fg_ok=%s drift=%d -> %s%s"
          % (label, len(rows),
             "n/a" if dda is None else "%.1f" % dda,
             "n/a" if fps is None else "%.1f" % fps,
             vis, fg_ok, drift, "PASS" if not problems else "FAIL",
             "" if not problems else "  (%s)" % "; ".join(problems)))
    return not problems


def main():
    if not os.path.exists(EXE):
        print("找不到被测 EXE: %s（先 cargo build --release，或用 VERRE_EXE 指定）" % EXE)
        return 2
    w, h = screen_size()
    print("verify_desktop_foreground: 主屏 %dx%d，被测 %s，沉降 %.1fs / 测量 %.1fs"
          % (w, h, EXE, SETTLE, MEASURE))
    print("  注意：测量期间请勿移动鼠标/键盘（前台与脏区都由脚本守住）")
    u.SetCursorPos(4, 4)
    probe = Probe()
    results = []
    try:
        make_ticker()
        pump(STARTUP)
        probe.drain()

        def exp_base(d, f, vis, dr):
            return (d is not None and d > 1.0 and vis,
                    "基线必须 dda>1 且画布可见（否则默认玻璃没建起来/被别的窗口盖住）")

        def exp_fs(d, f, vis, dr):
            # drift>0 时前台曾被抢走过（画布会短暂恢复可见，检查点可能撞在
            # 恢复瞬间），隐藏断言放宽；dda 与 fs_on marker 仍是硬判据。
            limit = 0.5 if dr == 0 else 1.0
            return (d is not None and d <= limit and (not vis or dr > 0),
                    "真全屏前台必须命中停产门：dda<=%.1f 且画布被隐藏（drift=%d）" % (limit, dr))

        def exp_keep(d, f, vis, dr):
            return (d is not None and d > 1.0 and vis,
                    "桌面壳前台时 dda 必须不降、画布必须可见（drift=%d）" % dr)

        results.append(phase(probe, "S0 baseline(无覆盖)", None, exp_base,
                             marks=[("fs_on", False)]))
        results.append(phase(
            probe, "S1 对照:真全屏必须停产",
            lambda: _popup("VerreFakeFS2", "verre fake exclusive fullscreen"),
            exp_fs, marks=[("fs_on", True)], require_fg=True))
        results.append(phase(probe, "S2 撤掉对照(恢复)", None, exp_base,
                             marks=[("fs_off", True)], keep_raw=True))
        results.append(phase(
            probe, "S3 假WorkerW不许停产",
            lambda: _popup("WorkerW", "verre fake workerw"),
            exp_keep, marks=[("fs_on", False)], require_fg=True))
        results.append(phase(
            probe, "S4 假Progman类名兜底",
            lambda: _popup("Progman", "verre fake progman"),
            exp_keep, marks=[("fs_on", False)], require_fg=True))
    finally:
        destroy(_COVER[0])
        destroy(_TICKER[0])
        probe.stop()

    n_fail = results.count(False)
    print("\n==============================================")
    if n_fail == 0:
        print("总体结论: PASS 桌面壳(WorkerW/Progman)不再误触发停产门, 真全屏对照组仍命中")
        return 0
    print("总体结论: FAIL %d 个阶段未通过" % n_fail)
    return 1


if __name__ == "__main__":
    sys.exit(main())
