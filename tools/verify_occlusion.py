# -*- coding: utf-8 -*-
"""verify_occlusion.py -- 遮挡剔除取证（A 隐藏屏停产 + B1 被遮挡块不进触发源）。

判据取自库自己打印的 `[metrics]` 行与翻转沿日志，不看截图 —— 因为 A 与 B1 都
**不改变单帧绘制内容**，要证明的是「该停的时候真的停了、该回来的时候真的回来了」。

阶段设计（每个阶段的状态翻转都由 **setup** 驱动，不用 teardown —— 否则恢复相的
翻转沿日志会落在上一阶段的窗口里，被 `del probe.raw[:]` 清掉，导致假 FAIL）：

  S0 baseline          不遮挡                              -> dda_acq > 0
  S1 B1 正向           不透明窗口完整覆盖                   -> dda_acq == 0，且有「桌面采集停产」日志
  S2 AC5 + 解除        换成 WS_EX_LAYERED + alpha=128       -> dda_acq 必须**回升**，且有「遮挡已移开」日志
                        （layered 窗口被 B1 明确否决，所以这一阶段同时证明「不许错剔」与「解除生效」）
  S3 干净基线          撤掉半透明窗口                       -> dda_acq > 0
  S4 A 正向（真实形态） 无边框铺满屏 + 前台                  -> dda_acq == 0、fps == 0、loop Hz 有上界，
                        且必须有「检测到独占全屏应用」日志
  S5 A 恢复            撤掉全屏窗口                         -> dda_acq > 0，且有「独占全屏应用已退出」日志
  S6 A 与 B1 解耦      **layered** 无边框铺满屏 + 前台       -> dda_acq == 0
                        ⚠ 这一阶段是 A 的**独占证明**：layered 窗口已被 S2 证明会被 B1 否决，
                          所以归零只可能来自 A 的停产门，不存在「其实是 B1 干的」这种归因歧义。
  S7 A 解耦恢复        撤掉 layered 全屏窗口                 -> dda_acq > 0

  `--off`（AC7 开关等价）只跑三阶段：不透明覆盖后 dda_acq **不许**下降（B1 完全失效），
  然后 S6/S7 照跑 —— A **没有开关**（它是「隐藏画布不该干活」的缺陷修复，不是可选优化）。

为什么每阶段要「沉降 + 测量」两段、且都 > 5 s：`[metrics]` 是 **5 秒墙钟窗口结算**
（`diag/metrics.rs` 的 `if secs < 5.0 { return; }`），跨越状态翻转的那一行是混合样本，必须丢掉。

为什么恢复阶段不能只断 `dda_acq > 0`：它证明不了玻璃真的回来了。`MonitorRt::ready` 的恢复
依赖采集真的交出一帧新桌面，`pass_a_mpx_avg > 0` 才证明 Pass A 在跑（玻璃在被画）；
再加上翻转沿日志，三者一起才算钉死。

**为什么「闪动桌面下 dda 整窗为 0」才是 A/B1 的铁证，而不是日志翻转沿**：
`fs_on` / `oc_on` 只能证明「门曾经合上」，不能证明「整个测量窗口都合着」。而覆盖窗口在闪动，
桌面持续有脏区，只要门开着采集就会立刻恢复 ⇒ `dda_acq` 在 5 秒窗口里中位数为 **0.0**
就等价于「这段时间基本没采过」。实测过反例：某轮 S4 里 `fs_on` 打完紧接着 `fs_off`
（前台被别的程序抢走），dda 归零其实是 B1 干的 —— 所以 A 的独占证明交给 S6
（**layered** 全屏窗口，B1 明确否决 layered，归零只能来自 A），日志只作辅助证据。
前台会被桌面不定时抢走（实测 95 秒内 6 次），脚本以 >=0.4 s 节流自动夺回并按阶段报告次数。

**覆盖窗口必须以 10 Hz 闪动**（两个只差 1 LSB 的深灰交替，且只闪屏幕正中 96x96 —— 
整窗失效等于每秒 6500 万像素的 GDI 填充，会把机器打成劣化窗口）。这是 `--off` 那轮实测逼出来的：
第一版探针窗口是静止的，铺满屏之后**桌面本身就安静了**，采集后端自然只交出约 1 帧/秒，
于是「开开关 0.0」与「关开关 1.0」几乎没有区分度 —— `dda_acq` 归零到底是剔除的功劳，
还是仅仅因为桌面没变化，说不清楚。让覆盖窗口持续产生脏区之后，桌面**一直在变**，
归零就只能来自「我们主动不采」，AC7 的反向对照才真的有牙齿。

测量期间**绝不移动鼠标**：`mouse_relevant` 只在 `cursor_moved` 时成立，
脚本启动即把光标停到左上角，否则会把 B1 的收益自己抵消掉。

已知环境依赖：本机若 `DuplicateOutput` 被拒（0x80070005）会自动回退 WGC，
`dda_acq` 字段统计的是「采集次数」，两条后端口径一致，判据不受影响。
"""
import ctypes
import io
import os
import re
import subprocess
import sys
import time
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
k32 = ctypes.windll.kernel32       # GetModuleHandleW / GetCurrentThreadId 在 kernel32

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# 被测可执行文件 = dev playground 产物；可用 VERRE_EXE 覆盖（例如指向旧基线做 A/B）。
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")
LOG = os.path.join(ROOT, "tools", "occlusion.log")

# --- Win32 常量 -------------------------------------------------------------
WM_DESTROY = 0x0002
WM_PAINT = 0x000F
WS_POPUP = 0x80000000
WS_VISIBLE = 0x10000000
WS_CAPTION = 0x00C00000
WS_EX_LAYERED = 0x00080000
HWND_TOPMOST = c_void_p(-1)
SWP_NOSIZE = 0x0001
SWP_NOMOVE = 0x0002
SWP_NOACTIVATE = 0x0010
SWP_SHOWWINDOW = 0x0040
LWA_ALPHA = 0x02
SM_CXSCREEN = 0
SM_CYSCREEN = 1
VK_MENU = 0xA4
KEYEVENTF_KEYUP = 0x0002
PM_REMOVE = 0x0001

# 沉降 / 测量时长：都必须 > 5 s（metrics 的结算窗口），见模块文档。
SETTLE = float(os.environ.get("VO_SETTLE", "5.4"))
MEASURE = float(os.environ.get("VO_MEASURE", "5.4"))
STARTUP = float(os.environ.get("VO_STARTUP", "6.0"))


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


# --- ctypes 原型 ------------------------------------------------------------
# 必须显式声明：ctypes 默认 restype 是 c_int，64 位下 CreateWindowExW 返回的 HWND 会被
# 截断（高 32 位丢掉），拿到一个无效句柄，后续 SetWindowPos / DestroyWindow 全部静默
# 失败 —— 那时探针窗口根本没盖住玻璃，脚本会误报「B1 没生效」。
u.CreateWindowExW.restype = c_void_p
u.CreateWindowExW.argtypes = [c_uint, c_void_p, ctypes.c_wchar_p, c_uint, c_int, c_int,
                              c_int, c_int, c_void_p, c_void_p, c_void_p, c_void_p]
u.DefWindowProcW.restype = ctypes.c_ssize_t
u.DefWindowProcW.argtypes = [c_void_p, c_uint, c_void_p, c_void_p]
u.RegisterClassW.restype = c_uint
u.RegisterClassW.argtypes = [ctypes.POINTER(WNDCLASSW)]
u.UnregisterClassW.argtypes = [ctypes.c_wchar_p, c_void_p]
u.BeginPaint.restype = c_void_p
u.BeginPaint.argtypes = [c_void_p, c_void_p]
u.EndPaint.argtypes = [c_void_p, c_void_p]
u.GetClientRect.argtypes = [c_void_p, ctypes.POINTER(RECT)]
u.SetWindowPos.restype = c_int
u.SetWindowPos.argtypes = [c_void_p, c_void_p, c_int, c_int, c_int, c_int, c_uint]
u.DestroyWindow.argtypes = [c_void_p]
u.UpdateWindow.argtypes = [c_void_p]
u.GetForegroundWindow.restype = c_void_p
u.SetForegroundWindow.restype = c_int
u.SetForegroundWindow.argtypes = [c_void_p]
u.BringWindowToTop.argtypes = [c_void_p]
u.GetWindowThreadProcessId.restype = c_uint
u.GetWindowThreadProcessId.argtypes = [c_void_p, c_void_p]
u.AttachThreadInput.restype = c_int
u.AttachThreadInput.argtypes = [c_uint, c_uint, c_int]
u.SetLayeredWindowAttributes.argtypes = [c_void_p, c_uint, ctypes.c_ubyte, c_uint]
u.PeekMessageW.argtypes = [ctypes.POINTER(MSG), c_void_p, c_uint, c_uint, c_uint]
u.TranslateMessage.argtypes = [ctypes.POINTER(MSG)]
u.DispatchMessageW.argtypes = [ctypes.POINTER(MSG)]
u.FillRect.argtypes = [c_void_p, ctypes.POINTER(RECT), c_void_p]   # FillRect 在 user32
u.InvalidateRect.argtypes = [c_void_p, c_void_p, c_int]
g.CreateSolidBrush.restype = c_void_p
g.CreateSolidBrush.argtypes = [c_uint]
g.DeleteObject.argtypes = [c_void_p]
k32.GetModuleHandleW.restype = c_void_p
k32.GetModuleHandleW.argtypes = [ctypes.c_wchar_p]
k32.GetCurrentThreadId.restype = c_uint

WNDPROC = WINFUNCTYPE(ctypes.c_ssize_t, c_void_p, c_uint, c_void_p, c_void_p)
_PAINT_BRUSH = None
_PAINT_BRUSH2 = None      # 闪动的第二色
_BLINK_HWND = [None]      # 当前需要闪动的探针窗口
_BLINK_ON = [0]
BLINK_HZ = 10.0
_FG_HOLD = [None]         # 需要**持续保持**前台的探针窗口（A 的判据依赖它）
_FG_DRIFT = [0]           # 前台被别的程序抢走的次数（实测：桌面会不定时机地抢焦点）
_FG_NEXT = [0.0]
_PHASE_DRIFT = [0]        # 本阶段内前台被抢走的次数（phase() 在调 expect 前写入）
LAST_DDA = [None]         # 上一个阶段测得的 dda_acq 中位数
BASELINE_DDA = [None]     # S0 基线，用来给「有漂移」的 A 阶段算相对门槛
_HINSTANCE = None
_CLASS = "VerreOcclusionProbe"
_CLASS_ATOM = None


def _wndproc(hwnd, msg, wp, lp):
    if msg == WM_PAINT:
        # 窗口必须**真的**被刷成实心色：AC5 的反向对照（半透明窗口不许被当成遮挡者）
        # 只有在「不透明窗口确实不透明」时才有意义。
        ps = (ctypes.c_byte * 136)()      # PAINTSTRUCT，尺寸富余即可，不读它的内容
        hdc = u.BeginPaint(c_void_p(hwnd), ctypes.byref(ps))
        if hdc:
            rc = RECT()
            u.GetClientRect(c_void_p(hwnd), ctypes.byref(rc))
            # 两色只差 1 LSB：肉眼几乎看不出闪烁，但每次重画都产生真实的像素变化，
            # 采集后端因此会持续报脏区（见模块文档「覆盖窗口必须闪动」）。
            brush = _PAINT_BRUSH2 if _BLINK_ON[0] else _PAINT_BRUSH
            u.FillRect(c_void_p(hdc), ctypes.byref(rc), brush)
        u.EndPaint(c_void_p(hwnd), ctypes.byref(ps))
        return 0
    if msg == WM_DESTROY:
        return 0
    # WM_ERASEBKGND 交给 DefWindowProc：它会用类画刷（hbrBackground）擦满整个窗口。
    # 自己返回 1 反而等于宣称「已擦」而什么都不画。
    return u.DefWindowProcW(c_void_p(hwnd), msg, c_void_p(wp), c_void_p(lp))


# 回调对象必须持有引用，否则被 GC 后 Windows 调进已释放内存 -> 崩溃。
_WNDPROC_REF = WNDPROC(_wndproc)


def wc_hinstance():
    global _HINSTANCE
    if _HINSTANCE is None:
        _HINSTANCE = k32.GetModuleHandleW(None)
    return _HINSTANCE


def _ensure_class():
    global _CLASS_ATOM, _PAINT_BRUSH
    if _CLASS_ATOM:
        return _CLASS_ATOM
    global _PAINT_BRUSH2
    _PAINT_BRUSH = g.CreateSolidBrush(0x00404040)     # COLORREF 是 0x00BBGGRR，这里是深灰
    _PAINT_BRUSH2 = g.CreateSolidBrush(0x00414141)    # 只差 1 LSB 的第二色
    wc = WNDCLASSW()
    wc.style = 0
    wc.lpfnWndProc = ctypes.cast(_WNDPROC_REF, c_void_p).value
    wc.hInstance = wc_hinstance()
    wc.hbrBackground = _PAINT_BRUSH
    wc.lpszClassName = _CLASS
    _CLASS_ATOM = u.RegisterClassW(ctypes.byref(wc))
    if not _CLASS_ATOM:
        raise RuntimeError("RegisterClassW failed")
    return _CLASS_ATOM


def screen_size():
    return u.GetSystemMetrics(SM_CXSCREEN), u.GetSystemMetrics(SM_CYSCREEN)


def _create(exstyle, style, title):
    atom = _ensure_class()
    w, h = screen_size()
    hwnd = u.CreateWindowExW(exstyle, atom, title, style, 0, 0, w, h,
                             None, None, wc_hinstance(), None)
    if not hwnd:
        raise RuntimeError("CreateWindowExW failed: %s" % title)
    # 被测画布默认就是 WS_EX_TOPMOST，不抬到 topmost 带就盖不住它。
    u.SetWindowPos(c_void_p(hwnd), HWND_TOPMOST, 0, 0, 0, 0,
                   SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW)
    u.UpdateWindow(c_void_p(hwnd))
    _BLINK_HWND[0] = hwnd
    return hwnd


def make_cover():
    """B1 的正向刺激：**不透明**、铺满屏、topmost，但**带标题栏**。
    带 WS_CAPTION 就绕开了 `fullscreen_app_on` 的判据（它要求 WS_POPUP 且无
    caption/thickframe），因此只触发 B1，不会顺手把 A 也触发掉。"""
    return _create(0, WS_POPUP | WS_CAPTION | WS_VISIBLE, "verre probe (cover)")


def make_cover_layered():
    """AC5 反向对照：几何与 make_cover 完全相同，只多一个 WS_EX_LAYERED + alpha=128。
    库的判据里 layered 窗口**一律不算遮挡者**（可能是整窗 alpha<255，也可能是逐像素 alpha），
    所以 dda_acq 不许降。"""
    hwnd = _create(WS_EX_LAYERED, WS_POPUP | WS_CAPTION | WS_VISIBLE, "verre probe (layered)")
    u.SetLayeredWindowAttributes(c_void_p(hwnd), 0, 128, LWA_ALPHA)
    return hwnd


def make_fullscreen(layered):
    """A 的刺激：WS_POPUP 无标题栏无可拖拽边框 + 铺满屏 + **必须是前台窗口**。
    `layered=True` 时同时带 WS_EX_LAYERED（alpha=255，视觉上仍完全不透明）——
    这样 B1 会明确否决它，`dda_acq` 归零就只可能来自 A，归因无歧义（S6）。"""
    exstyle = WS_EX_LAYERED if layered else 0
    hwnd = _create(exstyle, WS_POPUP | WS_VISIBLE, "verre probe (fullscreen)")
    if layered:
        u.SetLayeredWindowAttributes(c_void_p(hwnd), 0, 255, LWA_ALPHA)
    return hwnd


def destroy(hwnd):
    if hwnd:
        if _BLINK_HWND[0] == hwnd:
            _BLINK_HWND[0] = None
        if _FG_HOLD[0] == hwnd:
            _FG_HOLD[0] = None
        u.DestroyWindow(c_void_p(hwnd))


def force_foreground(hwnd):
    """把探针窗口设成前台窗口，并**自证**是否成功。

    `fullscreen_app_on` 的第一个判据就是 `GetForegroundWindow() == fg`，拿不到前台
    ⇒ A 根本不会被触发 ⇒ `dda_acq` 归零只能来自 B1，那是「测错了东西」，必须显式报出来
    （本机第一次跑就栽在这里：只敲 Alt 键不够，日志里连一条 `fs_on` 都没有）。
    因此除了 Alt 键技巧，再叠一层 `AttachThreadInput`：把本线程挂到当前前台线程的
    输入队列上，此时本进程被视为拥有前台权，`SetForegroundWindow` 才会成功；随后立刻解绑。
    """
    # 重试：本机实测前台争抢是**间歇**的 —— 同一份脚本连着跑，一次能拿到、下一次
    # 六轮都拿不到（前台应用一直在刷帧）。拿不到前台 ⇒ A 的判据不成立 ⇒ 整个阶段白测，
    # 所以这里多试几轮，比让脚本偶发假 FAIL 划算得多。
    got = False
    for _ in range(6):
        u.keybd_event(VK_MENU, 0, 0, 0)
        u.keybd_event(VK_MENU, 0, KEYEVENTF_KEYUP, 0)
        fg = u.GetForegroundWindow()
        fgtid = u.GetWindowThreadProcessId(c_void_p(fg), None) if fg else 0
        curtid = k32.GetCurrentThreadId()
        attached = bool(fgtid) and fgtid != curtid and bool(u.AttachThreadInput(fgtid, curtid, True))
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
        # 拿到还不够：必须**守住**。实测过一次 S4 里 `fs_on` 打完紧接着 `fs_off`
        # （前台被别的程序抢走），于是 dda 归零其实是 B1 干的，A 白测一轮。
        # pump() 会以 >=0.4 s 的间隔重新夺回前台并计数，漂移次数打进结果里。
        _FG_HOLD[0] = hwnd
    return got


def pump(seconds):
    """给自己的探针窗口跑消息泵（不泵就不画，窗口会是空的 => 不透明判据失真），
    同时以 BLINK_HZ 驱动它闪动，让桌面**持续**产生脏区（见模块文档）。"""
    end = time.perf_counter() + seconds
    next_blink = time.perf_counter()
    m = MSG()
    while time.perf_counter() < end:
        while u.PeekMessageW(ctypes.byref(m), None, 0, 0, PM_REMOVE):
            u.TranslateMessage(ctypes.byref(m))
            u.DispatchMessageW(ctypes.byref(m))
        now = time.perf_counter()
        # 守住前台：A 的判据 `fullscreen_app_on` 第一条就是 `GetForegroundWindow() == fg`，
        # 前台一丢 A 立刻失效。节流到 >=0.4 s 一次，避免和别的程序抢焦点抢成抖动。
        h_fg = _FG_HOLD[0]
        if h_fg and now >= _FG_NEXT[0]:
            _FG_NEXT[0] = now + 0.2
            if u.GetForegroundWindow() != h_fg:
                _FG_DRIFT[0] += 1
                u.SetForegroundWindow(c_void_p(h_fg))
                u.BringWindowToTop(c_void_p(h_fg))
        if now >= next_blink:
            next_blink = now + 1.0 / BLINK_HZ
            h = _BLINK_HWND[0]
            if h:
                _BLINK_ON[0] ^= 1
                # 只闪**屏幕正中一小块**，不整窗失效：
                # 整窗失效 = 每秒 3120x2080x10 ≈ 6500 万像素的 GDI 填充，实测把机器打成
                # 劣化窗口（loop_hz 掉到 2、max_loop 913 ms），测出来的数字不可信。
                # 只要桌面上**有任何**脏区，采集后端就会交帧，`dda_acq` 就 > 0；
                # 选正中是因为默认玻璃块居中 —— 脏区落在块足迹内，Pass A 才会真的跑，
                # `pass_a_mpx_avg > 0` 这条「玻璃确实被重画了」的断言才有信号。
                rc = RECT()
                w, hh = screen_size()
                rc.left, rc.top = w // 2 - 48, hh // 2 - 48
                rc.right, rc.bottom = w // 2 + 48, hh // 2 + 48
                # 第三个参数 0 = 不发 WM_ERASEBKGND，我们自己在 WM_PAINT 里填（会被裁剪到 rcPaint）
                u.InvalidateRect(c_void_p(h), ctypes.byref(rc), 0)
                u.UpdateWindow(c_void_p(h))
        time.sleep(0.02)


# --- metrics 解析 -----------------------------------------------------------
NUM = {
    "dda_acq": re.compile(rb"dda_acq=([0-9.]+)/s"),
    "fps": re.compile(rb"fps=([0-9.]+)"),
    "win_s": re.compile(rb"win_s=([0-9.]+)"),
    "loops": re.compile(rb"loops=([0-9]+)"),
    "blocks_avg": re.compile(rb"blocks_avg=([0-9.]+)"),
    "pass_a_mpx_avg": re.compile(rb"pass_a_mpx_avg=([0-9.]+)"),
    "max_loop_ms": re.compile(rb"max_loop_ms=([0-9.]+)"),
}

# `loop_ms_avg=msg../cap../prep..` 里的 prep：渲染前准备段 = **我们自己的 CPU 工作**
# （块投影 / 窗口区域 / 影响盒 / 遮挡扫描 / 快照发布），不含采集阻塞与 Present 背压。
PREP = re.compile(rb"loop_ms_avg=msg[\d.]+/cap[\d.]+/prep([\d.]+)")

# prep 段单圈中位数的预算（毫秒）。
# 这条断言是**实测逼出来的**：某一次 S1（B1 剔除期间）出现过 prep 单圈中位 3061 ms、
# `loops=2 / win_s=6.12`，即主循环被卡住数秒。后果不是「慢一点」，而是把「遮挡解除」的
# 检测延迟从设计的 ≤125 ms 拉到秒级 —— 玻璃里会长时间留着旧桌面。
# 同一刺激下单独重跑不复现（prep 0.039~0.078 ms、loops=598~600），当时桌面更忙
# （同一窗口 msg 段也飙到 415 ms），怀疑是 USER/GDI 全局锁竞争，但**归因未确认**。
# 所以把它变成常驻断言：复发就 FAIL，而不是等人肉眼发现。
# 20 ms = 实测基线（≤0.9 ms）的 20 倍以上余量；本脚本只用 1 块玻璃，
# 不存在 20 块 + SetWindowRgn 长尾那种合法尖峰。
PREP_BUDGET_MS = 20.0

# 库自己打的翻转沿日志（按 UTF-8 字节在原始日志行里搜）。
MARKS = {
    "fs_on": "检测到独占全屏应用，暂时隐藏玻璃".encode("utf-8"),
    "fs_off": "独占全屏应用已退出，恢复玻璃".encode("utf-8"),
    "oc_on": "桌面采集停产".encode("utf-8"),
    "oc_off": "遮挡已移开，恢复桌面采集".encode("utf-8"),
}


def read_metrics(path, offset):
    """从 offset 起读新追加的字节，返回 (新的 offset, [metrics 样本], [原始日志行])。
    用二进制读：Rust 的 stderr logger 写 UTF-8，而本机 chcp=936，按文本读会踩编码。"""
    try:
        with io.open(path, "rb") as fh:
            fh.seek(offset)
            data = fh.read()
            offset = fh.tell()
    except OSError:
        return offset, [], []
    rows, lines = [], []
    for line in data.split(b"\n"):
        if not line.strip():
            continue
        lines.append(line)
        if b"[metrics]" not in line:
            continue
        s = {}
        for k, rx in NUM.items():
            m = rx.search(line)
            if m:
                s[k] = float(m.group(1))
        m = PREP.search(line)
        if m:
            s["prep_ms"] = float(m.group(1))
        if s:
            rows.append(s)
    return offset, rows, lines


def med(vals):
    if not vals:
        return None
    v = sorted(vals)
    return v[len(v) // 2]


class Probe(object):
    def __init__(self, occlusion):
        env = dict(os.environ)
        # 库默认**不**自建玻璃窗口（上层应用负责 create_window）；取证脚本需要一块
        # 确定性的默认玻璃，因此显式打开（与 tools/ 其余脚本同一约定）。
        env["GLASS_DEFAULT_BLOCK"] = "1"
        env["GLASS_MONITORS"] = "1"
        env["GLASS_METRICS"] = "1"
        # 计数等价证明不允许自适应降档中途改行为，其余 verify_* 脚本同样关掉。
        env["GLASS_ADAPTIVE"] = "0"
        env["GLASS_OCCLUSION"] = "1" if occlusion else "0"
        for k in ("GLASS_MANY", "GLASS_FAR_GRID", "GLASS_SPREAD", "GLASS_SIM_BACKDROP",
                  "GLASS_SIM_BUSY", "GLASS_CURSOR_AT", "GLASS_INTERNAL_SHOT",
                  "GLASS_INTERNAL_SHOT_AT_MS", "GLASS_VERIFY_VALIDITY"):
            env.pop(k, None)
        if os.path.exists(LOG):
            os.remove(LOG)
        self.fh = io.open(LOG, "wb")
        self.p = subprocess.Popen([EXE, "--no-browser"], env=env, cwd=ROOT,
                                  stdout=subprocess.DEVNULL, stderr=self.fh)
        self.off = 0
        self.raw = []       # 本阶段（含沉降期）收集到的原始日志行

    def drain(self):
        self.off, rows, lines = read_metrics(LOG, self.off)
        self.raw.extend(lines)
        return rows

    def sample(self, seconds):
        pump(seconds)
        return self.drain()

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


def phase(probe, label, expect, setup=None, marks=(), keep_raw=False):
    """跑一个阶段：setup 翻转状态 -> 沉降（丢掉跨越翻转的 metrics 窗口）-> 测量 -> 判定。

    ⚠ 状态翻转**只能**由 setup 驱动，不要用 teardown：恢复相的翻转沿日志是在
    「撤掉遮挡窗口」那一刻打的，若放在上一阶段的 teardown 里，它会在本阶段
    `del probe.raw[:]` 之前落盘，于是被清掉 => 假 FAIL（本机实测踩过）。

    `expect(dda, fps, loop_hz, pass_a_mpx, n_samples) -> (bool, why)`
    `marks` 是 `[(MARKS 的键, 是否必须出现)]`，在**沉降 + 测量**全程的日志里搜：
    翻转沿日志是在沉降期打的（遮挡确认要连续 2 轮 = ≤250 ms），只在测量窗口搜会漏。
    """
    # keep_raw=True：沿用上一阶段的日志窗口。恢复阶段（S5/S7）需要它 ——
    # 前台被抢时 `hidden_fs` 会在**上一个**阶段里就来回翻转，`fs_off` 那条日志
    # 于是落在上一阶段的窗口里；只在本阶段窗口里搜会假 FAIL。
    # 语义上我们要断言的是「翻转确实发生过」，不是「翻转必须发生在这 10.8 秒内」。
    if not keep_raw:
        del probe.raw[:]                # 从状态翻转这一刻起收集日志（含沉降期）
    drift0 = _FG_DRIFT[0]
    if setup:
        setup()
    pump(SETTLE)
    probe.drain()                       # 丢掉跨越翻转的那个 metrics 窗口；日志留在 raw 里
    # 采到至少 1 个样本为止（上限 3 倍预算）：`[metrics]` 只在主循环回到
    # `report_metrics` 时才结算，机器劣化时一个 5 s 窗口可能要 8~10 s 才吐得出来
    # （实测 loop_hz=2 那一轮 S0 就是 0 样本 -> 假 FAIL）。
    rows = []
    deadline = time.perf_counter() + MEASURE * 3.0
    while not rows and time.perf_counter() < deadline:
        rows.extend(probe.sample(2.0))
    dda = med([r["dda_acq"] for r in rows if "dda_acq" in r])
    fps = med([r["fps"] for r in rows if "fps" in r])
    pa = med([r["pass_a_mpx_avg"] for r in rows if "pass_a_mpx_avg" in r])
    prep = med([r["prep_ms"] for r in rows if "prep_ms" in r])
    maxloop = med([r["max_loop_ms"] for r in rows if "max_loop_ms" in r])
    loops = med([r["loops"] for r in rows if "loops" in r])
    win_s = med([r["win_s"] for r in rows if "win_s" in r]) or 5.0
    hz = (loops / win_s) if loops is not None else None
    drift = _FG_DRIFT[0] - drift0
    _PHASE_DRIFT[0] = drift
    LAST_DDA[0] = dda
    ok, why = expect(dda, fps, hz, pa, len(rows))
    problems = [] if ok else [why]
    # prep 预算：与具体阶段的期望无关，所有阶段都必须成立（见 PREP_BUDGET_MS 注释）。
    # max_loop_ms 只打印不断言 —— 它含 Present 的 vsync/DWM 背压，基线本来就有
    # 100~180 ms 的合法长尾，拿它当门槛会 flaky。
    if prep is not None and prep > PREP_BUDGET_MS:
        problems.append("prep 段单圈中位 %.0f ms 超预算 %.0f ms（主循环被卡住，"
                        "遮挡解除的检测延迟会从 125 ms 拉到秒级）" % (prep, PREP_BUDGET_MS))
    for key, required in marks:
        found = any(MARKS[key] in ln for ln in probe.raw)
        if found != required:
            problems.append("日志%s「%s」翻转沿" % ("缺少" if required else "出现了不该有的", key))
    ok = not problems
    if drift:
        print("       (%s: 前台被抢 %d 次并已夺回；A 的判据依赖前台，"
              "但 dda 整窗为 0 仍说明停产占了绝大部分时间)" % (label, drift))
    print("  %-22s n=%d dda=%s fps=%s passA=%s loop_hz=%s prep_ms=%s max_loop=%s -> %s%s"
          % (label, len(rows),
             "n/a" if dda is None else "%.1f" % dda,
             "n/a" if fps is None else "%.1f" % fps,
             "n/a" if pa is None else "%.2f" % pa,
             "n/a" if hz is None else "%.0f" % hz,
             "n/a" if prep is None else "%.2f" % prep,
             "n/a" if maxloop is None else "%.0f" % maxloop,
             "PASS" if ok else "FAIL",
             "" if ok else "  (%s)" % "; ".join(problems)))
    return ok


def main():
    if not os.path.exists(EXE):
        print("找不到被测 EXE: %s（先 cargo build --release，或用 VERRE_EXE 指定）" % EXE)
        return 2
    off_mode = "--off" in sys.argv
    occlusion = not off_mode
    w, h = screen_size()
    print("verify_occlusion: 主屏 %dx%d，被测 %s，GLASS_OCCLUSION=%s，沉降 %.1fs / 测量 %.1fs"
          % (w, h, EXE, "1" if occlusion else "0", SETTLE, MEASURE))
    print("  注意：测量期间请勿移动鼠标、勿动键盘（光标一动就给 mouse_relevant 送触发源）")
    if off_mode:
        print("  （--off：AC7 开关等价 —— B1 必须完全失效；A 的开关是 GLASS_FS_HIDE，"
              "本脚本刻意不设，所以 S6/S7 仍须过）")

    # 光标停到左上角并全程不动：`mouse_relevant` 只在 cursor_moved 时成立。
    # 小屏（1920x1080）上这个点仍可能落在默认玻璃的影响盒内，但只要光标不动就不触发。
    u.SetCursorPos(4, 4)
    probe = Probe(occlusion)
    results = []
    hwnd = [None]
    fg_ok = [None]

    def swap(maker, want_fg=False):
        def go():
            destroy(hwnd[0])
            hwnd[0] = maker()
            if want_fg:
                fg_ok[0] = force_foreground(hwnd[0])
        return go

    def drop():
        def go():
            destroy(hwnd[0])
            hwnd[0] = None
        return go

    def fg_gate(rest_of_why):
        """A 阶段的门槛，**漂移感知**。

        前台没拿到 => `fullscreen_app_on` 的第一条判据不成立 => 这一阶段等于没测，
        必须显式 FAIL 而不是含糊通过。

        前台拿到了但中途被抢过（实测桌面会不定时抢焦点，95 秒内最多 9 次）=>
        `hidden_fs` 会来回翻转，严格 `dda == 0` 就不成立了。这时退一档：要求 dda
        显著低于基线（<= max(0.5, 0.25*基线)），并把降级原因打进结果里 ——
        既不放过「A 根本没生效」（那会是 dda ≈ 基线），也不因为桌面抢焦点而假 FAIL。
        桌面安静（零漂移）时仍然是严格的 `dda <= 0.5`。
        """
        def expect(d, f, hz, pa, n):
            if fg_ok[0] is False:
                return False, ("SetForegroundWindow 没拿到前台，fullscreen_app_on 不可能命中（"
                               + rest_of_why + "）")
            base = BASELINE_DDA[0] or 0.0
            limit = 0.5 if _PHASE_DRIFT[0] == 0 else max(0.5, 0.25 * base)
            ok = (d is not None and d <= limit and f is not None and f <= max(1.0, 0.25 * base)
                  and hz is not None and hz <= 200.0)
            why = ("dda<=%.1f 且 fps≈0 且 loop Hz<=200（不退化成忙等）；基线 dda=%.1f，"
                   "本阶段前台被抢 %d 次" % (limit, base, _PHASE_DRIFT[0]))
            return ok, why
        return expect

    try:
        pump(STARTUP)
        probe.drain()

        results.append(phase(
            probe, "S0 baseline 无遮挡",
            lambda d, f, hz, pa, n: (n > 0 and d is not None and d > 1.0,
                                     "基线就该有采集；若为 0，说明默认玻璃没建起来，"
                                     "或者它本来就被别的窗口盖住了"),
            marks=[("oc_on", False), ("fs_on", False)]))

        BASELINE_DDA[0] = LAST_DDA[0] or 0.0
        print("  基线 dda_acq = %.1f/s（A 阶段在「前台被抢」时按 0.25*基线 放宽门槛）"
              % BASELINE_DDA[0])

        if occlusion:
            results.append(phase(
                probe, "S1 B1 不透明全覆盖",
                lambda d, f, hz, pa, n: (d is not None and d <= 0.5,
                                         "被完整覆盖后 dda_acq 必须归零"),
                setup=swap(make_cover), marks=[("oc_on", True)]))
            # 换半透明窗口 = 同时验「layered 不许被当成遮挡者」与「遮挡解除后采集回来」
            results.append(phase(
                probe, "S2 AC5 半透明+解除",
                lambda d, f, hz, pa, n: (d is not None and d > 1.0 and pa is not None and pa > 0.0,
                                         "layered 窗口不许被当成遮挡者，dda_acq 必须回升且 Pass A 在跑"),
                setup=swap(make_cover_layered), marks=[("oc_off", True), ("oc_on", False)]))
            results.append(phase(
                probe, "S3 撤掉半透明",
                lambda d, f, hz, pa, n: (d is not None and d > 1.0, "应回到干净基线"),
                setup=drop(), marks=[("oc_on", False)]))
            results.append(phase(
                probe, "S4 A 全屏(真实形态)",
                fg_gate("此时 dda 归零也可能来自 B1，归因不成立"),
                setup=swap(lambda: make_fullscreen(False), True),
                marks=[("fs_on", True)]))
            results.append(phase(
                probe, "S5 A 退出全屏",
                lambda d, f, hz, pa, n: (d is not None and d > 1.0 and pa is not None and pa > 0.0,
                                         "全屏退出后必须恢复采集，且 Pass A 在跑"),
                setup=drop(), marks=[("fs_off", True)], keep_raw=True))
        else:
            results.append(phase(
                probe, "S1 AC7 关开关不剔除",
                lambda d, f, hz, pa, n: (d is not None and d > 1.0,
                                         "GLASS_OCCLUSION=0 时不得停采"),
                setup=swap(make_cover), marks=[("oc_on", False)]))
            results.append(phase(
                probe, "S3 撤掉覆盖窗口",
                lambda d, f, hz, pa, n: (d is not None and d > 1.0, "应回到干净基线"),
                setup=drop(), marks=[("oc_on", False)]))

        # S6/S7：A 的**独占**证明 —— layered 全屏窗口已被 S2 证明会被 B1 否决，
        # 所以这里的 dda 归零只可能来自 A 的停产门，不存在归因歧义。
        results.append(phase(
            probe, "S6 A 全屏(layered 解耦)",
            fg_gate("B1 已排除，只剩前台这一条前提"),
            setup=swap(lambda: make_fullscreen(True), True),
            marks=[("fs_on", True)]))
        results.append(phase(
            probe, "S7 A 解耦恢复",
            lambda d, f, hz, pa, n: (d is not None and d > 1.0 and pa is not None and pa > 0.0,
                                     "全屏退出后必须恢复采集，且 Pass A 在跑"),
            setup=drop(), marks=[("fs_off", True)], keep_raw=True))
    finally:
        destroy(hwnd[0])
        probe.stop()
        try:
            u.UnregisterClassW(_CLASS, wc_hinstance())
        except Exception:
            pass
        if _PAINT_BRUSH:
            g.DeleteObject(_PAINT_BRUSH)

    total = len(results)
    passed = sum(1 for r in results if r)
    if _FG_DRIFT[0]:
        print("  前台被其它程序抢走 %d 次（已自动夺回）；次数很多说明桌面在打架，"
              "A 的阶段结论要打折看" % _FG_DRIFT[0])
    print("verify_occlusion: %d/%d PASS -> %s" % (passed, total,
                                                 "PASS" if passed == total else "FAIL"))
    return 0 if passed == total else 1


if __name__ == "__main__":
    sys.exit(main())
