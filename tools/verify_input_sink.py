# -*- coding: utf-8 -*-
"""verify_input_sink.py -- GlassInputSink 输入路由取证（PRD AC3 五条行为断言）。

物理输入注入（ctypes SetCursorPos/mouse_event，verify_dirty.py 同门手法）；滚轮/双击
用 PostMessage 直达画布（WM_MOUSEWHEEL 物理注入只送前台窗口，NOACTIVATE 画布不保证收到）。
行为判定读 [verify] 日志：

  断言 1  Consumed 区（左半）按压拖动 -> 块矩形不动 + Down dec=Consumed
  断言 2  DragMove 区（右半）按压拖动 -> 块随手移动（对照组：1/2 同果则脚本自判 FAIL）
  断言 3  RightDown/RightUp/DoubleClick/Wheel(±1) 全部到达 sink
  断言 4  把手带在 sink=Consumed 的那半仍可缩放（把手优先，不询问 sink）
  断言 5  capture 归属：断言 1 按住期间 cap=0（不抓鼠标）；断言 2 按住期间 cap=画布

路由实现若被删，[verify] attach/state 日志整体消失 => 脚本在源头 FAIL，不会假 PASS。
"""
import ctypes
import os
import re
import subprocess
import sys
import threading
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")

user32 = ctypes.windll.user32
user32.SetProcessDPIAware()

LEFTDOWN, LEFTUP, RIGHTDOWN, RIGHTUP = 0x0002, 0x0004, 0x0008, 0x0010
WM_LBUTTONDBLCLK, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_MOUSEWHEEL = 0x0203, 0x0204, 0x0205, 0x020A


class POINT(ctypes.Structure):
    _fields_ = [("x", ctypes.c_long), ("y", ctypes.c_long)]


def base_env():
    e = os.environ.copy()
    # hermetic，与 verify_dirty/cmp_visual 家族一致：确定性单块、关自适应降档、
    # 不让独占全屏检测把画布藏掉。
    e["GLASS_DEFAULT_BLOCK"] = "1"
    e["GLASS_INPUT_SINK_DEMO"] = "1"
    e["GLASS_FS_HIDE"] = "0"
    e["GLASS_MONITORS"] = "1"
    e["GLASS_ADAPTIVE"] = "0"
    return e


class Session:
    """一个 playground 进程 + 后台日志收集线程（教训：readline 阻塞必须分线程）。"""

    def __init__(self, name, wait_state_s=14.0):
        self.name = name
        self.lines = []
        self.lock = threading.Lock()
        self.proc = subprocess.Popen(
            [EXE, "--no-browser", "--no-demo-pair"], env=base_env(), cwd=ROOT,
            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
            text=True, encoding="utf-8", errors="replace")
        threading.Thread(target=self._pump, daemon=True).start()
        deadline = time.time() + wait_state_s
        while time.time() < deadline and self.last_state() is None:
            time.sleep(0.15)

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
            m = re.search(
                r"sink_demo_state id=\d+ rect=\[(-?[\d.]+),\s*(-?[\d.]+),\s*(-?[\d.]+),\s*(-?[\d.]+)\] scale=([\d.]+)",
                line)
            if m:
                return tuple(float(g) for g in m.groups())
        return None

    def lines_with(self, needle):
        return [l for l in self.snapshot() if needle in l]

    def count(self, needle):
        return len(self.lines_with(needle))

    def cap_values(self, phase_needle):
        out = []
        for l in self.lines_with(phase_needle):
            m = re.search(r"cap=(-?\d+)", l)
            if m:
                out.append(int(m.group(1)))
        return out

    def close(self):
        self.proc.terminate()
        try:
            self.proc.wait(timeout=4)
        except subprocess.TimeoutExpired:
            self.proc.kill()


def park_cursor():
    sh = user32.GetSystemMetrics(1)
    user32.SetCursorPos(8, sh - 8)


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


def find_canvas(cx, cy):
    """TRANSPARENT 逐帧切换 => WindowFromPoint 要多次采样取众数（非 0）。"""
    votes = {}
    for _ in range(24):
        user32.SetCursorPos(int(cx), int(cy))
        time.sleep(0.05)
        hw = user32.WindowFromPoint(POINT(int(cx), int(cy)))
        if hw:
            votes[hw] = votes.get(hw, 0) + 1
        time.sleep(0.02)
    if not votes:
        return 0
    return max(votes, key=votes.get)


def make_lparam(x, y):
    return (int(y) << 16) | (int(x) & 0xFFFF)


RESULTS = []


def check(name, ok, detail):
    print(("  PASS: " if ok else "  FAIL: ") + name + " -- " + detail)
    RESULTS.append(bool(ok))
    return ok


def geom(sess):
    """返回 (逻辑x,y,w,h,scale, 物理px,py,pw,ph)。"""
    st = sess.last_state()
    if st is None:
        return None
    x, y, w, h, scale = st[0], st[1], st[2], st[3], st[4]
    return x, y, w, h, scale, x * scale, y * scale, w * scale, h * scale


def summarize():
    n_fail = RESULTS.count(False)
    print("\n==============================================")
    if n_fail == 0:
        print("总体结论: PASS 五条行为断言全过（attach/state 是接线证据；拖动/缩放/cap 是决策链路证据）")
        return 0
    print("总体结论: FAIL %d 条断言未通过" % n_fail)
    return 1


def main():
    print("EXE = %s" % EXE)
    if not os.path.isfile(EXE):
        print("FAIL: 被测 EXE 不存在（先 cargo build --release）")
        return 1

    # ---- 断言 1 + 5a：Consumed 区不建会话、不抓 capture ----
    print("\n--- 断言 1/5a: Consumed 区按压拖动块不动且不抓 capture ---")
    sA = Session("consumed")
    try:
        g = geom(sA)
        if not check("sink demo 挂载且 rect 心跳存在", g is not None, "sink_demo_state 可见"):
            return summarize()
        x, y, w, h, scale, px, py, pw, ph = g
        park_cursor()
        time.sleep(1.0)
        # 左半按下，一路拖过右半：Down 已被消费，粘滞规则下越界也不得开会话
        drag(px + pw * 0.25, py + ph * 0.5, pw * 0.45, 0)
        park_cursor()
        time.sleep(1.2)
        g2 = geom(sA)
        moved = abs(g2[0] - x) + abs(g2[1] - y)
        resized = abs(g2[2] - w) + abs(g2[3] - h)
        check("Consumed 区按压拖动块不动（含越界粘滞）",
              moved <= 2.0 and resized <= 2.0,
              "Δpos=%.1f Δsize=%.1f（阈值 ≤2 逻辑px）" % (moved, resized))
        downs = sA.lines_with("phase=Down")
        check("Down 被询问且返回 Consumed", any("dec=Consumed" in l for l in downs),
              "Down 事件 %d 条" % len(downs))
        caps = sA.cap_values("phase=Move")
        check("按住期间 Move 全程 cap=0（未抓 capture）",
              len(caps) > 0 and all(c == 0 for c in caps),
              "Move %d 条 cap!=0 计数=%d" % (len(caps), sum(1 for c in caps if c != 0)))
    finally:
        sA.close()

    # ---- 断言 2 + 5b：DragMove 区会话照旧、capture=画布 ----
    print("\n--- 断言 2/5b: DragMove 区按压拖动块随手移动，按住期间 cap=画布 ---")
    sB = Session("dragmove")
    try:
        g = geom(sB)
        if g is None:
            check("sink demo 心跳存在", False, "B 轮未见 state")
            return summarize()
        x, y, w, h, scale, px, py, pw, ph = g
        park_cursor()
        time.sleep(1.0)
        dx_phys = min(160.0, pw * 0.35)
        drag(px + pw * 0.75, py + ph * 0.5, dx_phys, 0)
        park_cursor()
        time.sleep(1.2)
        g2 = geom(sB)
        exp_dx = dx_phys / scale
        moved_dx = g2[0] - x
        resized = abs(g2[2] - w) + abs(g2[3] - h)
        check("DragMove 区按压拖动块随输入移动（对照断言1）",
              abs(moved_dx - exp_dx) <= 35.0 and resized <= 35.0,
              "dx=%.1f 期望≈%.1f±35 Δsize=%.1f" % (moved_dx, exp_dx, resized))
        downs = sB.lines_with("phase=Down")
        check("Down 被询问且返回 DragMove", any("dec=DragMove" in l for l in downs),
              "Down 事件 %d 条" % len(downs))
        # 画布句柄在块「新位置」中心采样（鼠标下窗口=画布）
        cx = (g2[0] + g2[2] * 0.5) * scale
        cy = (g2[1] + g2[3] * 0.5) * scale
        hwnd = find_canvas(cx, cy)
        caps = sB.cap_values("phase=Move")
        n_own = sum(1 for c in caps if c == hwnd and hwnd != 0)
        check("按住期间 Move 的 cap=画布 HWND（capture 生效）",
              hwnd != 0 and n_own >= 4,
              "cap==hwnd 的 Move %d 条（>=4 判定按住期间），hwnd=%d" % (n_own, hwnd))
    finally:
        sB.close()

    # ---- 断言 4：把手带优先（左半角也是缩放）----
    print("\n--- 断言 4: 把手带在 Consumed 半区仍可缩放（不询问 sink）---")
    sC = Session("resize")
    try:
        g = geom(sC)
        if g is None:
            check("sink demo 心跳存在", False, "C 轮未见 state")
            return summarize()
        x, y, w, h, scale, px, py, pw, ph = g
        park_cursor()
        time.sleep(1.0)
        # 左下角（块内左半区域）：若询问 sink 会得到 Consumed；把手带恒优先 => 应缩放
        drag(px + 6.0, py + ph - 6.0, -90.0, 90.0)
        park_cursor()
        time.sleep(1.2)
        g2 = geom(sC)
        exp = 90.0 / scale
        dw = g2[2] - w
        dh = g2[3] - h
        dxr = g2[0] - x
        check("把手带按压拖动改变尺寸（左下外扩）",
              dw >= exp * 0.5 and dh >= exp * 0.5 and dxr <= -exp * 0.5 and abs(g2[1] - y) <= 8.0,
              "dw=%.0f dh=%.0f dx=%.0f 期望各≈%.0f（半宽容忍）" % (dw, dh, dxr, exp))
        check("把手带路径未询问 sink（规则1：无 Down 决策日志）",
              sC.count("phase=Down") == 0, "Down 事件=%d" % sC.count("phase=Down"))
    finally:
        sC.close()

    # ---- 断言 3：特殊事件到达 ----
    print("\n--- 断言 3: RightDown/RightUp/DoubleClick/Wheel(±1) 到达 sink ---")
    sD = Session("events")
    try:
        g = geom(sD)
        if g is None:
            check("sink demo 心跳存在", False, "D 轮未见 state")
            return summarize()
        x, y, w, h, scale, px, py, pw, ph = g
        cx = (x + w * 0.6) * scale
        cy = (y + h * 0.5) * scale
        user32.SetCursorPos(int(cx), int(cy))
        time.sleep(0.4)
        hwnd = find_canvas(cx, cy)
        if not check("事件轮拿到画布句柄（WindowFromPoint 众数）", hwnd != 0, "hwnd=%d" % hwnd):
            return summarize()
        # 右键：真实按钮消息走鼠标下窗口
        user32.mouse_event(RIGHTDOWN, 0, 0, 0, 0)
        time.sleep(0.12)
        user32.mouse_event(RIGHTUP, 0, 0, 0, 0)
        time.sleep(0.15)
        # 滚轮：PostMessage 直达（前台不是画布，物理滚轮不保证送达）
        user32.PostMessageW(hwnd, WM_MOUSEWHEEL, (120 & 0xFFFF) << 16, make_lparam(cx, cy))
        time.sleep(0.12)
        user32.PostMessageW(hwnd, WM_MOUSEWHEEL, ((-120) & 0xFFFF) << 16, make_lparam(cx, cy))
        time.sleep(0.12)
        # 双击：先物理快速两次，再 PostMessage 兜底（只要到达即可）
        user32.mouse_event(LEFTDOWN, 0, 0, 0, 0)
        time.sleep(0.03)
        user32.mouse_event(LEFTUP, 0, 0, 0, 0)
        time.sleep(0.04)
        user32.mouse_event(LEFTDOWN, 0, 0, 0, 0)
        time.sleep(0.03)
        user32.mouse_event(LEFTUP, 0, 0, 0, 0)
        time.sleep(0.08)
        # 物理连击的 DBLCLK 合成依赖系统双击时限，不稳定；直接 PostMessage 兜底
        # （dispatch 读 read_cursor，不读 lp，坐标不受影响）。
        user32.PostMessageW(hwnd, WM_LBUTTONDBLCLK, 0x0001, make_lparam(cx, cy))
        time.sleep(0.15)
        park_cursor()
        time.sleep(0.8)
        n_rd = sD.count("phase=RightDown")
        n_ru = sD.count("phase=Up(Some(Right))")
        n_dbl = sD.count("phase=DoubleClick")
        wpos = [l for l in sD.lines_with("phase=Wheel") if "wheel=1" in l]
        wneg = [l for l in sD.lines_with("phase=Wheel") if re.search(r"wheel=-1\b", l)]
        check("RightDown 到达", n_rd >= 1, "n=%d" % n_rd)
        check("RightUp（Up(Some(Right))）到达", n_ru >= 1, "n=%d" % n_ru)
        check("DoubleClick 到达", n_dbl >= 1, "n=%d" % n_dbl)
        check("Wheel ±1 各到达一次", len(wpos) >= 1 and len(wneg) >= 1,
              "pos=%d neg=%d" % (len(wpos), len(wneg)))
    finally:
        sD.close()

    return summarize()


if __name__ == "__main__":
    sys.exit(main())
