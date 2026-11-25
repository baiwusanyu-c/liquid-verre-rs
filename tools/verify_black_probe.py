# -*- coding: utf-8 -*-
"""verify_black_probe.py -- 采集黑帧探针的状态机 / 翻转裁决 / 自动降级闭环（0.5.4）。

背景（宿主定位报告 2026-11，Win10 + Wallpaper Engine）：
  路径 A：WGC 会把「被 WDA_EXCLUDEFROMCAPTURE 排除的自家全屏画布」整幅涂黑
          （依 Windows 版本 / 驱动而异；本机 Win10 的 WGC 实测**不涂黑**，
          所以无法用真实触发做常驻断言）；
  路径 B：MPO 叠加层不进复制帧（WE 视频壁纸）。

因此本脚本用 `GLASS_BLACK_PROBE=force`（把「画布仍在排除」的采样帧一律视为
全黑，摘除排除后的帧仍按真实内容裁决）**仿真触发整条状态机**，对每个阶段断言
的都是「机制」而不是「某个 OS 是否恰好涂黑」：采样 → 翻转 → digest 裁决 →
降级 / 只告警 → warn-once。真实的「某系统 WGC 确实涂黑」由同一代码路径处理，
差别只在第一步的 all_black 是真测出来的。

阶段（各自独立起 EXE，因为「后端选择 + 探针武装」都发生在会话建立期）：

  R1 真实桌面(默认后端)        不许出现任何「黑帧探针」日志（本机桌面非全黑，
                              探针必须判健康即停，不许误报/误换端）
  R2 force + 强制 wgc          必须出现「已自动降级 DDA 采集」，随后新 DDA 会话
                              被重新武装并打「黑帧探针：DDA 连续」告警；画布全程可见
  R3 强制 wgc + 探针关(=0)     不许出现「黑帧探针」日志（取证后门有效性）
  R4 force + 默认 dda          必须出现「黑帧探针：DDA 连续」告警，且**不许**出现
                              「降级」（DDA 全黑与纯黑桌面不可分辨 ⇒ 只提示不换端）

判别力：把 runtime 的探针接线删掉 ⇒ R2/R4 缺 marker 红；把采样阈值/翻转裁决的
保守方向写反 ⇒ R1 出现 marker 红；把 GLASS_BLACK_PROBE=0 的短路删掉 ⇒ R3 红；
把「DDA 不换端」写错 ⇒ R4 出现降级红。

⚠ 测量期间鼠标钉左上角不动；闪动小窗负责让桌面持续有真实脏区（没有内容帧，
探针的采样/裁决帧根本不会到，脚本会 FAIL 在缺 marker —— 这是有意的：
「凑不齐样本」必须可见，而不是静默跳过）。
"""
import ctypes
import io
import os
import re
import subprocess
import sys
import time

# 控制台 GBK 时中文可写而特殊符号（如 ⇒）会 UnicodeEncodeError：统一转 UTF-8 兜底。
if hasattr(sys.stdout, "reconfigure"):
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass
from ctypes import c_void_p, c_int, c_uint, c_long, Structure, WINFUNCTYPE

try:
    ctypes.windll.user32.SetProcessDpiAwarenessContext(c_void_p(-4))
except Exception:
    try:
        ctypes.windll.user32.SetProcessDPIAware()
    except Exception:
        pass

u = ctypes.windll.user32
g = ctypes.windll.gdi32

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")
LOG = os.path.join(ROOT, "tools", "black_probe.log")
CANVAS_CLASS = "LiquidVerreCanvas"

WM_PAINT = 0x000F
WM_DESTROY = 0x0002
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
PM_REMOVE = 0x0001

STARTUP = float(os.environ.get("VP_STARTUP", "7.0"))
RUN = float(os.environ.get("VP_RUN", "14.0"))


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
u.CreateWindowExW.argtypes = [c_uint, ctypes.c_wchar_p, ctypes.c_wchar_p, c_uint,
                              c_int, c_int, c_int, c_int, c_void_p, c_void_p, c_void_p, c_void_p]
u.DefWindowProcW.restype = ctypes.c_ssize_t
u.DefWindowProcW.argtypes = [c_void_p, c_uint, c_void_p, c_void_p]
u.RegisterClassW.restype = c_uint
u.RegisterClassW.argtypes = [ctypes.POINTER(WNDCLASSW)]
u.SetWindowPos.restype = c_int
u.SetWindowPos.argtypes = [c_void_p, c_void_p, c_int, c_int, c_int, c_int, c_uint]
u.DestroyWindow.argtypes = [c_void_p]
u.UpdateWindow.argtypes = [c_void_p]
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
u.GetWindowThreadProcessId.restype = c_uint
u.GetWindowThreadProcessId.argtypes = [c_void_p, c_void_p]
u.GetSystemMetrics.restype = c_int
u.GetSystemMetrics.argtypes = [c_int]
u.SetCursorPos.restype = c_int
u.SetCursorPos.argtypes = [c_int, c_int]
g.CreateSolidBrush.restype = c_void_p
g.CreateSolidBrush.argtypes = [c_uint]
k32 = ctypes.windll.kernel32
k32.GetModuleHandleW.restype = c_void_p
k32.GetModuleHandleW.argtypes = [ctypes.c_wchar_p]

_BRUSH = g.CreateSolidBrush(0x00404040)
_BRUSH2 = g.CreateSolidBrush(0x00414141)
_ON = [0]
_TICKER = [None]
_HINST = [None]

WNDPROC = WINFUNCTYPE(ctypes.c_ssize_t, c_void_p, c_uint, c_void_p, c_void_p)


def _wndproc(hwnd, msg, wp, lp):
    if msg == WM_PAINT:
        ps = (ctypes.c_byte * 136)()
        hdc = u.BeginPaint(c_void_p(hwnd), ctypes.byref(ps))
        if hdc:
            rc = RECT()
            u.GetClientRect(c_void_p(hwnd), ctypes.byref(rc))
            u.FillRect(c_void_p(hdc), ctypes.byref(rc), _BRUSH2 if _ON[0] else _BRUSH)
        u.EndPaint(c_void_p(hwnd), ctypes.byref(ps))
        return 0
    return u.DefWindowProcW(c_void_p(hwnd), msg, c_void_p(wp), c_void_p(lp))


_PROC_REF = WNDPROC(_wndproc)


def pump(seconds):
    end = time.perf_counter() + seconds
    nxt = time.perf_counter()
    while time.perf_counter() < end:
        m = MSG()
        while u.PeekMessageW(ctypes.byref(m), None, 0, 0, PM_REMOVE):
            u.TranslateMessage(ctypes.byref(m))
            u.DispatchMessageW(ctypes.byref(m))
        now = time.perf_counter()
        if now >= nxt:
            nxt = now + 0.1
            _ON[0] ^= 1
            if _TICKER[0]:
                u.InvalidateRect(c_void_p(_TICKER[0]), None, 0)
                u.UpdateWindow(c_void_p(_TICKER[0]))
        time.sleep(0.02)


def make_ticker():
    if _TICKER[0]:
        return _TICKER[0]
    wc = WNDCLASSW()
    wc.lpfnWndProc = ctypes.cast(_PROC_REF, c_void_p).value
    _HINST[0] = k32.GetModuleHandleW(None)
    wc.hInstance = _HINST[0]
    wc.hbrBackground = _BRUSH
    wc.lpszClassName = "VerreTicker"
    u.RegisterClassW(ctypes.byref(wc))
    w, h = u.GetSystemMetrics(SM_CXSCREEN), u.GetSystemMetrics(SM_CYSCREEN)
    hwnd = u.CreateWindowExW(WS_EX_TOPMOST, "VerreTicker", "verre ticker",
                             WS_POPUP | WS_VISIBLE, max(0, w - 150), max(0, h - 150),
                             96, 96, None, None, _HINST[0], None)
    if not hwnd:
        raise RuntimeError("ticker create failed")
    u.SetWindowPos(c_void_p(hwnd), HWND_TOPMOST, 0, 0, 0, 0,
                   SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW)
    _TICKER[0] = hwnd
    return hwnd


DDA_RX = re.compile(rb"dda_acq=([0-9.]+)/s")
PROBE = "黑帧探针".encode("utf-8")
DOWNGRADE = "已自动降级 DDA 采集".encode("utf-8")
DDA_WARN = "黑帧探针：DDA 连续".encode("utf-8")


def read_lines(path):
    if not os.path.exists(path):
        return []
    with io.open(path, "rb") as fh:
        return [ln for ln in fh.read().split(b"\n") if ln.strip()]


def canvases(pid):
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


def run_case(label, extra_env, must=(), must_not=()):
    """起一个专属 EXE 进程，跑 RUN 秒，按日志 marker + 画布可见性 + dda 连续性判定。"""
    env = dict(os.environ)
    env["GLASS_DEFAULT_BLOCK"] = "1"
    env["GLASS_MONITORS"] = "1"
    env["GLASS_METRICS"] = "1"
    env["GLASS_ADAPTIVE"] = "0"
    env["GLASS_OCCLUSION"] = "0"
    for k in ("GLASS_FS_HIDE", "GLASS_BACKEND", "GLASS_BLACK_PROBE", "GLASS_MANY",
              "GLASS_FAR_GRID", "GLASS_SPREAD", "GLASS_SIM_BACKDROP", "GLASS_SIM_BUSY",
              "GLASS_CURSOR_AT", "GLASS_INTERNAL_SHOT", "GLASS_SYNTH_CAP"):
        env.pop(k, None)
    env.update(extra_env)
    if os.path.exists(LOG):
        os.remove(LOG)
    problems = []
    fh = io.open(LOG, "wb")
    p = subprocess.Popen([EXE, "--no-browser"], env=env, cwd=ROOT,
                         stdout=subprocess.DEVNULL, stderr=fh)
    try:
        make_ticker()
        pump(STARTUP)
        pump(RUN)
        fh.flush()
        lines = read_lines(LOG)
        blob = b"\n".join(lines)
        for needle in must:
            if needle not in blob:
                problems.append("缺少日志「%s」" % needle.decode("utf-8"))
        for needle in must_not:
            if needle in blob:
                problems.append("出现了不该有的日志「%s」" % needle.decode("utf-8"))
        # 画布必须**全程可见**：探针只换后端/告警，隐藏画布是越权（黑壁纸误伤红线）。
        vis = any(u.IsWindowVisible(c_void_p(h)) for h in canvases(p.pid))
        if not vis:
            problems.append("画布不可见（探针绝不允许隐藏画布）")
        # 采集必须活着（R2 的降级链路最终要交得出帧）。
        ddas = [float(DDA_RX.search(ln).group(1))
                for ln in lines if DDA_RX.search(ln)]
        if not ddas or max(ddas) <= 0.0:
            problems.append("全程 dda_acq=0（采集没跑起来或探针把采集打死了）")
    finally:
        try:
            p.terminate()
            p.wait(timeout=10)
        except Exception:
            try:
                p.kill()
            except Exception:
                pass
        fh.close()
    print("  %-34s dda=%s -> %s%s"
          % (label, ddas[-1] if ddas else "n/a",
             "PASS" if not problems else "FAIL",
             "" if not problems else "  (%s)" % "; ".join(problems)))
    return not problems


def main():
    if not os.path.exists(EXE):
        print("找不到被测 EXE: %s（先 cargo build --release，或用 VERRE_EXE 指定）" % EXE)
        return 2
    print("verify_black_probe: 被测 %s，单阶段 %.0fs" % (EXE, STARTUP + RUN))
    print("  注意：测量期间请勿动鼠标/键盘")
    u.SetCursorPos(4, 4)
    results = []
    try:
        results.append(run_case(
            "R1 真实桌面(探针开)不误报", {},
            must=(), must_not=[PROBE]))
        results.append(run_case(
            "R2 force+强制wgc -> 降级闭环",
            {"GLASS_BACKEND": "wgc", "GLASS_BLACK_PROBE": "force"},
            must=[DOWNGRADE, DDA_WARN], must_not=[]))
        results.append(run_case(
            "R3 探针关(=0) 全程静默",
            {"GLASS_BACKEND": "wgc", "GLASS_BLACK_PROBE": "0"},
            must=[], must_not=[PROBE]))
        results.append(run_case(
            "R4 force+dda -> 只告警不换端",
            {"GLASS_BACKEND": "dda", "GLASS_BLACK_PROBE": "force"},
            must=[DDA_WARN], must_not=[DOWNGRADE]))
    finally:
        if _TICKER[0]:
            u.DestroyWindow(c_void_p(_TICKER[0]))

    n_fail = results.count(False)
    print("\n==============================================")
    if n_fail == 0:
        print("总体结论: PASS 黑帧探针(采样/翻转/裁决/降级/只告警/取证后门)全部闭环")
        return 0
    print("总体结论: FAIL %d 个阶段未通过" % n_fail)
    return 1


if __name__ == "__main__":
    sys.exit(main())
