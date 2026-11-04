#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""verify_grip_occlusion.py -- 0.5.3 #9/#10：下层框的边缘不许在被压住时亮起来。

主诉（宿主 2026-11-04 复验）：0.5.2 #1 修对了 border 的**绘制层序**，没修**触发判据** ——
两块重叠、上层块 B 的机身盖住下层块 A 的左边缘中段时，光标放进那段「B 的机身」里：
A 照样亮缩放反馈白边（被 B 盖住的那段看不见，**两头露出来**）、手型照样变缩放箭头、
点下去又拉不动。根因：`hit_resize` 只在**把手候选之间**比 z 序，而 B 在该点根本不是候选
（光标不在 B 的边缘带里）⇒ B 连参与比较的机会都没有。

oracle 用**状态**不用像素（`WindowSnapshot::hovered`）：它就是本条缺陷的直接判据，
比像素稳定，也不与 `verify_border_zorder.py`（管绘制）重叠。

断言（§4 的「删掉过滤句它还红吗」逐条对应）：
  [1] p_covered（A 左边缘带 ∩ B 机身）：A.hovered == false          ← 主判据
  [2] p_free   （A 左边缘带 ∩ B 之外）：A.hovered == true           ← 反向对照，防修过头
  [3] covered → free → covered 来回：两轮都收敛，无粘滞              ← 每帧轮询的复位正确性
  [4] #10：在 p_free 上盖一个**外部**顶层窗口 ⇒ A.hovered == false；
      撤掉窗口 ⇒ 下一轮又回到 true（带撤除对照，排除「脚本把环境弄坏了」的解释）

场景由 playground 的 `GLASS_GRIP_DEMO=1` 构造并**物理**移动光标；本脚本只读日志 +
盖/撤外部窗口，不第二处手写布局。

⚠ 本脚本**不设** `GLASS_CURSOR_AT`：#10 的门控查的是真实光标，钉死模式按设计绕过它
（`runtime::mod` 里有注释说明为什么必须绕过），一钉死 #10 就测成空转。

用法：
  python tools/verify_grip_occlusion.py
  VERRE_EXE=<exe> 覆盖被测二进制。
"""
import ctypes
import ctypes.wintypes
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
    ctypes.windll.user32.SetProcessDPIAware()
u = ctypes.windll.user32

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")
TMP = os.path.join(tempfile.gettempdir(), "verre_grip_occlusion")
os.makedirs(TMP, exist_ok=True)

HERMETIC = {
    "GLASS_FS_HIDE": "0",
    "GLASS_MONITORS": "1",
    "GLASS_ADAPTIVE": "0",
    "GLASS_DEFAULT_BLOCK": "1",
    "GLASS_SYNTH_CAP": "1",
    "GLASS_ENV_EMA": "0",
    "GLASS_GRIP_DEMO": "1",
}

SCENE = re.compile(
    r"grip_scene a=\[([^\]]+)\] b=\[([^\]]+)\] grip=([\d.]+) "
    r"p_covered=(-?\d+),(-?\d+) p_free=(-?\d+),(-?\d+)"
)
PROBE = re.compile(
    r"grip_probe round=(\d+) pt=(\w+) a_id=(\d+) a_hovered=(true|false) hovered_ids=\[([\d,\s]*)\]"
)


def ids_of(text):
    return [int(v) for v in text.split(",") if v.strip()]


class Fail(Exception):
    pass


def read_log(path):
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""


class CoverWindow:
    """一个**外部**不透明顶层窗口，用来验证 #10 的 WindowFromPoint 门控。

    用系统已注册的 `Static` 类 ⇒ 不需要 RegisterClassW；创建线程自己跑一个
    PeekMessage 泵，免得窗口被判「未响应」影响命中测试。
    """

    WS_POPUP = 0x80000000
    WS_VISIBLE = 0x10000000
    WS_EX_TOPMOST = 0x00000008
    WS_EX_TOOLWINDOW = 0x00000080
    HWND_TOPMOST = -1
    SWP_SHOWWINDOW = 0x0040
    SW_HIDE = 0

    def __init__(self, x, y, w=64, h=64):
        self.hwnd = None
        self.stop = threading.Event()
        self.box = (int(x - w / 2), int(y - h / 2), w, h)
        self.thread = threading.Thread(target=self._pump, daemon=True)
        self.thread.start()

    def _pump(self):
        # GetModuleHandleW 在 **kernel32**（不在 user32）；0.5.3 首跑踩过：
        # AttributeError: function 'GetModuleHandleW' not found。
        k32 = ctypes.windll.kernel32
        # 64 位下必须显式声明返回类型，否则 HWND 被默认 c_int 截断成 32 位
        # —— 截断后的句柄传给 SetWindowPos/DestroyWindow 就是未定义行为。
        k32.GetModuleHandleW.restype = ctypes.c_void_p
        k32.GetModuleHandleW.argtypes = [ctypes.c_wchar_p]
        u.CreateWindowExW.restype = ctypes.c_void_p
        u.CreateWindowExW.argtypes = [
            ctypes.c_ulong, ctypes.c_wchar_p, ctypes.c_wchar_p, ctypes.c_ulong,
            ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
            ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p,
        ]
        u.SetWindowPos.restype = ctypes.c_int
        u.SetWindowPos.argtypes = [
            ctypes.c_void_p, ctypes.c_void_p, ctypes.c_int, ctypes.c_int,
            ctypes.c_int, ctypes.c_int, ctypes.c_uint,
        ]
        u.ShowWindow.restype = ctypes.c_int
        u.ShowWindow.argtypes = [ctypes.c_void_p, ctypes.c_int]
        u.DestroyWindow.restype = ctypes.c_bool
        u.DestroyWindow.argtypes = [ctypes.c_void_p]
        inst = k32.GetModuleHandleW(None)
        cls = ctypes.c_wchar_p("Static")
        title = ctypes.c_wchar_p("verre-grip-cover")
        self.hwnd = u.CreateWindowExW(
            self.WS_EX_TOPMOST | self.WS_EX_TOOLWINDOW, cls, title,
            self.WS_POPUP | self.WS_VISIBLE,
            self.box[0], self.box[1], self.box[2], self.box[3],
            None, None, inst, None,
        )
        msg = ctypes.wintypes.MSG()
        while not self.stop.is_set():
            try:
                while u.PeekMessageW(ctypes.byref(msg), 0, 0, 0, 1):
                    u.TranslateMessage(ctypes.byref(msg))
                    u.DispatchMessageW(ctypes.byref(msg))
            except Exception:
                pass
            # 反复重申 topmost：画布本身也在 topmost 带里，必须压过它。
            u.SetWindowPos(self.hwnd, self.HWND_TOPMOST, self.box[0], self.box[1],
                           self.box[2], self.box[3], self.SWP_SHOWWINDOW)
            time.sleep(0.02)
        u.ShowWindow(self.hwnd, self.SW_HIDE)
        u.DestroyWindow(self.hwnd)

    def close(self):
        self.stop.set()
        self.thread.join(timeout=2.0)


def wait_probes(path, count, timeout=30.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        found = PROBE.findall(read_log(path))
        if len(found) >= count:
            return found
        time.sleep(0.2)
    raise Fail("只等到 %d 条 grip_probe（需要 %d 条），日志 %s"
               % (len(PROBE.findall(read_log(path))), count, path))


def main():
    if not os.path.exists(EXE):
        print("FAIL  找不到被测二进制 %s（先 cargo build --release --workspace）" % EXE)
        return 1
    print("[grip-occlusion] 被测 = %s" % EXE)
    env = dict(os.environ)
    env.update(HERMETIC)
    log_path = os.path.join(TMP, "run.log")
    if os.path.exists(log_path):
        os.unlink(log_path)
    with open(log_path, "wb") as logf:
        p = subprocess.Popen([EXE, "--no-browser", "--no-demo-pair"], env=env, cwd=ROOT,
                             stdout=subprocess.DEVNULL, stderr=logf)
    results = []
    cover = None
    try:
        scene, deadline = None, time.time() + 20.0
        while time.time() < deadline and scene is None:
            m = SCENE.search(read_log(log_path))
            if m:
                scene = m.groups()
            time.sleep(0.1)
        if not scene:
            raise Fail("没解析到 [verify] grip_scene（日志 %s）" % log_path)
        p_covered = (int(scene[3]), int(scene[4]))
        p_free = (int(scene[5]), int(scene[6]))
        print("  A=%s B=%s grip=%s p_covered=%s p_free=%s"
              % (scene[0], scene[1], scene[2], p_covered, p_free))

        # ---- 断言 1~3：前三轮采样（covered / free / covered2）----
        probes = wait_probes(log_path, 3)
        first3 = probes[:3]
        for r in first3:
            print("    round=%s pt=%s a_id=%s a_hovered=%s hovered_ids=%s"
                  % (r[0], r[1], r[2], r[3], ids_of(r[4])))
        pts = [x[1] for x in first3]
        if pts != ["covered", "free", "covered2"]:
            raise Fail("采样序列不是 covered/free/covered2：%s" % pts)
        a_id = int(first3[0][2])
        others = [i for i in ids_of(first3[0][4]) if i != a_id]

        # 断言 1 是**两条**：A 不许亮，而且归属必须已经上移到 B ——
        # 只查「A 不亮」会把「谁都不亮」也判成通过，而清单 §2.5 点名的正是
        # 「Move 转发给上层块、亮边亮在下层块」这种自相矛盾。
        ok1 = first3[0][3] == "false" and bool(others)
        print("  [1] p_covered：A 不亮 且 归属已上移到上层块 %s -> %s"
              % (others, "PASS" if ok1 else "FAIL"))
        results.append(ok1)

        ok2 = first3[1][3] == "true" and ids_of(first3[1][4]) == [a_id]
        print("  [2] p_free：A 亮且只有 A 亮（防修过头）-> %s" % ("PASS" if ok2 else "FAIL"))
        results.append(ok2)

        ok3 = first3[2][3] == "false"
        print("  [3] 回到 p_covered 仍不亮（无粘滞）-> %s" % ("PASS" if ok3 else "FAIL"))
        results.append(ok3)

        # ---- 断言 4：#10 外部窗口遮挡 p_free ⇒ A 不亮；撤掉后恢复 ----
        # 等到下一轮 free 采样之前把窗口盖上，再等两次 free 采样：
        #   盖上期间的 free   -> 期望 false
        #   撤掉之后的 free   -> 期望 true（撤除对照）
        seen_before = len(PROBE.findall(read_log(log_path)))
        cover = CoverWindow(p_free[0], p_free[1])
        time.sleep(0.6)
        probes2 = wait_probes(log_path, seen_before + 4, timeout=30.0)
        # 撤窗。⚠ 撤掉之后必须睡过 demo 那一步的 400 ms 采样窗，否则「撤窗之后」的
        # 第一条样本可能仍落在窗口还活着的那半个周期里 —— 0.5.3 首跑就是这样把
        # after 测成 ['true','false','false']：判据看着过了，其实「true」来自盖窗**之前**，
        # 整条断言是空证据。
        seen_at_close = len(PROBE.findall(read_log(log_path)))
        cover.close()
        cover = None
        time.sleep(0.6)
        probes3 = wait_probes(log_path, seen_at_close + 4, timeout=30.0)

        # `wait_probes` 返回的是**全部**已出现的 probe（日志只追加），所以按
        # 「盖窗前的条数」和「撤窗时的条数」切三段，每段只含自己时期的样本。
        before = [r[3] for r in probes2[:seen_before] if r[1] == "free"]
        during = [r[3] for r in probes2[seen_before:seen_at_close] if r[1] == "free"]
        after = [r[3] for r in probes3[seen_at_close:] if r[1] == "free"]
        print("    盖窗前 free 采样: %s" % before)
        print("    盖窗期间 free 采样: %s" % during)
        print("    撤窗之后 free 采样: %s" % after)
        # 判据：盖窗期间的**最后**一条必须 false（门控生效），撤窗后**必须观察到**恢复
        # 成 true。为什么看 after 的末条而不是首条：#10 的门控只在光标**移动过**时重查
        # （清单 §6 的成本建议，库注释同口径），撤窗那一刻光标还停在 p_free 上没动，
        # 所以撤窗后第一条可能仍停在「上一次判定 = 不在画布上」里，要等 demo 下一步
        # covered -> free 的光标移动才纠正。实测：撤窗后 = ['false','true']。
        # 这条时效语义是**已知设计**，不是缺陷：外来窗口盖上来时同样要等一次光标移动
        # 才灭边；宿主复验时请按「动一下鼠标」为基准，别拿静止鼠标当反例。
        ok4 = (
            bool(during) and len(after) >= 2
            and during[-1] == "false"
            and after[-1] == "true"
        )
        print("  [4] 盖窗期间不亮（during 末条=%s），撤窗后恢复（after 末条=%s，共 %d 条）-> %s"
              % (during[-1] if during else "无", after[-1] if after else "无", len(after),
                 "PASS" if ok4 else "FAIL"))
        results.append(ok4)
    finally:
        if cover is not None:
            cover.close()
        if p.poll() is None:
            p.terminate()
            try:
                p.wait(timeout=5)
            except Exception:
                p.kill()

    bad = results.count(False)
    print("总体:", "ALL PASS" if bad == 0 else "FAIL(%d/%d)" % (bad, len(results)))
    return 0 if bad == 0 else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Fail as e:
        print("FAIL  %s" % e)
        sys.exit(1)