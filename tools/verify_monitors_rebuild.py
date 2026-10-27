# -*- coding: utf-8 -*-
"""verify_monitors_rebuild.py -- 显示器变化重建路径取证（缺陷 #1/#2/#4 行为面，AC1/AC4）。

手法：向画布 PostMessageW(WM_DISPLAYCHANGE) 触发 wnd_proc 置位 rebuild_monitors（与真实
热插拔/改分辨率走同一条代码路径，但不触碰系统显示配置 —— 取证不能改用户桌面）。

断言集（全部由重建这个刺激驱动，任何一条缺证据即 FAIL）：
  C1  按类名枚举到画布（GLASS_MONITORS=1 时 1 块）。
  C2  刺激后旧画布 HWND 在 5s 内 IsWindow=False —— 证明重建真的发生（不是空转）。
  C3  刺激后出现新的画布 HWND（与旧集合不相交）。
  C4  刺激后 8s 进程仍存活且没有「引擎已退出」—— 缺陷 #1 的直接反面：
      旧实现 WM_DESTROY 无条件 PostQuitMessage，C2 成立的同一瞬间渲染线程必死。
  C5  刺激后仍有新的 [metrics] 行（主循环活着）与新的 sink_demo_state 行（块仍在投影）。
  C6  重建后块 rect 与重建前一致（<=0.5 逻辑px）—— 缺陷 #4 的行为钉。判别力依赖
      本机 scale != 1.0：旧实现把 b.scale 强清 1.0 后按新 scale 重乘，200% 屏上宽度
      直接翻倍（宿主 2026-10-27 上午「2x 膨胀」现场）。100% 缩放下此断言退化为恒真，
      输出行会标注「判别力：无」提醒（#4 仍由 core/block.rs 单测矩阵钉死）。
  C7  全程日志不出现 RegisterClassExW 失败 —— 缺陷 #2 在重建路径的回归位。

已知口径：GLASS_MONITORS=1 只限启动期；rebuild_monitors 重枚举**全部**显示器，所以
刺激后画布数可能 1 -> N（本机 2）。脚本记录数量但不作相等断言。
"""
import ctypes
import os
import re
import subprocess
import threading
import time
from ctypes import c_void_p, c_uint, c_int

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")

user32 = ctypes.windll.user32
try:
    user32.SetProcessDpiAwarenessContext(c_void_p(-4))  # PerMonitorV2，坐标口径与库一致
except Exception:
    user32.SetProcessDPIAware()

WM_DISPLAYCHANGE = 0x007E

# ctypes 原型显式声明：默认 restype=c_int 在 64 位下会截断 HWND（verify_occlusion 前科）。
user32.FindWindowExW.restype = c_void_p
user32.FindWindowExW.argtypes = [c_void_p, c_void_p, ctypes.c_wchar_p, ctypes.c_wchar_p]
user32.GetWindowThreadProcessId.restype = c_uint
user32.GetWindowThreadProcessId.argtypes = [c_void_p, ctypes.POINTER(c_uint)]
user32.GetDpiForWindow.restype = c_uint
user32.GetDpiForWindow.argtypes = [c_void_p]
user32.IsWindow.restype = c_int
user32.IsWindow.argtypes = [c_void_p]
user32.PostMessageW.restype = c_int
user32.PostMessageW.argtypes = [c_void_p, c_uint, c_void_p, c_void_p]
CANVAS_CLASS = "LiquidVerreCanvas"
RESULTS = []


def check(name, ok, detail):
    print(("  PASS: " if ok else "  FAIL: ") + name + " -- " + detail)
    RESULTS.append(bool(ok))
    return ok


class Session:
    """playground 进程 + 双管道日志收集（stdout 抓引擎退出播报，stderr 是库日志）。"""

    def __init__(self):
        env = os.environ.copy()
        # hermetic 家族口径：确定性单块、单屏、静止节拍可测、关自适应、关全屏让路。
        env["GLASS_DEFAULT_BLOCK"] = "1"
        env["GLASS_MONITORS"] = "1"
        env["GLASS_METRICS"] = "1"
        env["GLASS_ADAPTIVE"] = "0"
        env["GLASS_FS_HIDE"] = "0"
        env["GLASS_INPUT_SINK_DEMO"] = "1"
        for k in ("GLASS_MANY", "GLASS_FAR_GRID", "GLASS_SPREAD", "GLASS_SIM_BACKDROP",
                  "GLASS_SIM_BUSY", "GLASS_CURSOR_AT", "GLASS_INTERNAL_SHOT",
                  "GLASS_INTERNAL_SHOT_AT_MS", "GLASS_VERIFY_VALIDITY", "GLASS_OCCLUSION"):
            env.pop(k, None)
        self.proc = subprocess.Popen(
            [EXE, "--no-browser", "--no-demo-pair"], env=env, cwd=ROOT,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, encoding="utf-8", errors="replace")
        self.out, self.err = [], []
        self.lock = threading.Lock()
        threading.Thread(target=self._pump, args=(self.proc.stdout, self.out), daemon=True).start()
        threading.Thread(target=self._pump, args=(self.proc.stderr, self.err), daemon=True).start()

    def _pump(self, stream, sink):
        try:
            for line in stream:
                with self.lock:
                    sink.append(line.rstrip())
        except Exception:
            pass

    def snapshot(self):
        with self.lock:
            return list(self.out), list(self.err)

    def alive(self):
        return self.proc.poll() is None

    def close(self):
        self.proc.terminate()
        try:
            self.proc.wait(timeout=6)
        except subprocess.TimeoutExpired:
            self.proc.kill()


STATE_RX = re.compile(
    r"sink_demo_state id=\d+ rect=\[(-?[\d.]+),\s*(-?[\d.]+),\s*(-?[\d.]+),\s*(-?[\d.]+)\] scale=([\d.]+)")


def last_state(sess):
    _, err = sess.snapshot()
    for line in reversed(err):
        m = STATE_RX.search(line)
        if m:
            return tuple(float(g) for g in m.groups())
    return None


def count_metrics(sess):
    _, err = sess.snapshot()
    return sum(1 for l in err if "[metrics]" in l)


def count_state_lines(sess):
    _, err = sess.snapshot()
    return sum(1 for l in err if "sink_demo_state" in l)


def find_canvas_all(pid=None):
    """按类名链式枚举画布；pid 给定时只要该进程的窗口。

    必须按 pid 过滤：上一轮取证残留的孤儿画布（缺陷家族前科：GLASS_MONITORS truncate
    泄漏）同进程/跨进程都可能混进类名枚举，PostMessage 打中孤儿 = 刺激静默蒸发。
    """
    out = []
    h = 0
    while True:
        h = user32.FindWindowExW(0, h, CANVAS_CLASS, None)
        if not h:
            break
        if pid is not None:
            got = ctypes.c_uint()
            user32.GetWindowThreadProcessId(ctypes.c_void_p(h), ctypes.byref(got))
            if got.value != pid:
                continue
        out.append(int(h))
    return out


def canvas_dpi(hwnd):
    try:
        return int(user32.GetDpiForWindow(ctypes.c_void_p(hwnd)))
    except Exception:
        return 0


def summarize():
    n_fail = RESULTS.count(False)
    print("\n==============================================")
    if n_fail == 0:
        print("总体结论: PASS 重建存活 + rect 重估 + 类注册回归全部钉住")
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
    while time.time() < deadline and last_state(sess) is None:
        time.sleep(0.2)
    g0 = last_state(sess)
    if not check("启动成功且 sink rect 心跳可见", g0 is not None, str(g0)):
        sess.close()
        return summarize()
    canvases = find_canvas_all(sess.proc.pid)
    if not check("C1 枚举到本进程画布", len(canvases) >= 1, "canvases=%s pid=%d" % (canvases, sess.proc.pid)):
        sess.close()
        return summarize()
    old_set = set(canvases)
    metrics0 = count_metrics(sess)
    state0 = count_state_lines(sess)

    # 对本进程全部画布广播刺激：活动画布命中即触发 rebuild；未绑定 USERDATA 的
    # 孤儿窗口对消息无动作（app_of=None），广播不产生副作用。
    print("\n--- 刺激：PostMessage(WM_DISPLAYCHANGE) x %d canvases ---" % len(canvases))
    t_stim = time.time()
    for hw in canvases:
        user32.PostMessageW(c_void_p(hw), WM_DISPLAYCHANGE, 0, 0)

    # C2/C3：5s 内旧画布销毁 + 新画布出现
    old_dead_at, dead_hwnd, new_set = None, None, set()
    while time.time() - t_stim < 5.0:
        if old_dead_at is None:
            for hw in sorted(old_set):
                if not user32.IsWindow(c_void_p(hw)):
                    old_dead_at, dead_hwnd = time.time() - t_stim, hw
                    break
        new_set = set(find_canvas_all(sess.proc.pid))
        if old_dead_at is not None and (new_set - old_set):
            break
        time.sleep(0.05)
    if not check("C2 旧画布被重建销毁（IsWindow=False）", old_dead_at is not None,
                 "dead=0x%x t=%.2fs 仍存活旧句柄=%s" % (
                     dead_hwnd or 0, old_dead_at if old_dead_at else -1.0,
                     sorted(new_set & old_set))):
        sess.close()
        return summarize()
    new_only = sorted(new_set - old_set)
    if not check("C3 出现新的画布 HWND", len(new_only) >= 1,
                 "new=%s old=%s" % (new_only, sorted(old_set))):
        sess.close()
        return summarize()
    print("  info: 重建后画布数 %d（GLASS_MONITORS=1 只限启动期，rebuild 重枚举全部屏）"
          % len(new_set))

    # C4/C5：再观察 8s —— 进程活着、metrics 与 state 都在追加、无「引擎已退出」
    time.sleep(8.0)
    alive = sess.alive()
    out1, err1 = sess.snapshot()
    died_log = [l for l in out1 if "引擎已退出" in l]
    check("C4 刺激 8s 后进程存活且无「引擎已退出」", alive and not died_log,
          "alive=%s exit_lines=%d" % (alive, len(died_log)))
    metrics1 = count_metrics(sess)
    state1 = count_state_lines(sess)
    check("C5a 重建后 [metrics] 继续结算", metrics1 > metrics0,
          "metrics %d -> %d" % (metrics0, metrics1))
    check("C5b 重建后 sink rect 心跳继续", state1 > state0 + 1,
          "state 行数 %d -> %d" % (state0, state1))

    # C6：块 rect 与重建前一致（#4 判别力：任一新画布 DPI != 96 时才有牙齿。
    # 判据 = max(GetDpiForWindow) 跨全部新画布 —— 枚举顺序可能把副屏（96 dpi）
    # 排在首位，只看 [0] 会把有牙齿的场景误报成无判别力。
    g1 = last_state(sess)
    live = find_canvas_all(sess.proc.pid)
    dpis = [canvas_dpi(hw) for hw in live]
    dpis = [d for d in dpis if d]
    scale = (max(dpis) / 96.0) if dpis else g0[4]
    dx = dy = dw = dh = -1.0
    if g1 is not None:
        dx = abs(g1[0] - g0[0])
        dy = abs(g1[1] - g0[1])
        dw = abs(g1[2] - g0[2])
        dh = abs(g1[3] - g0[3])
    same = g1 is not None and dx <= 0.5 and dy <= 0.5 and dw <= 0.5 and dh <= 0.5
    teeth = ("有——scale>1 时旧实现强清 b.scale 会致尺寸重乘"
             if scale > 1.05 else
             "无——100% 缩放下退化为恒真，#4 由 core/block.rs 单测矩阵钉死")
    check("C6 重建后块 rect 不变（判别力：%s）" % teeth, same,
          "dpos=(%.1f,%.1f) dsize=(%.1f,%.1f) 阈值<=0.5 scale=%.2f" % (dx, dy, dw, dh, scale))

    # C7：无 RegisterClassExW 失败（#2 在重建路径的回归位）
    bad = [l for l in err1 if "RegisterClassExW" in l]
    check("C7 全程无 RegisterClassExW 失败日志", not bad, "命中 %d 条" % len(bad))

    sess.close()
    return summarize()


if __name__ == "__main__":
    import sys
    sys.exit(main())

