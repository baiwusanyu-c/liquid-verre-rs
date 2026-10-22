# -*- coding: utf-8 -*-
"""ab_memory.py -- 常驻内存受控 A/B（轮转交错 + 中位数）。

为什么需要它：本机 Private Bytes 在**同一进程内**就能漂 50 MB（verify_adaptive
实测 level 0 基线 1.5~3s = 237 MB、44~50s = 187 MB），单次运行的 max/median
不足以支撑任何结论。WDDM 下 GPU 分配计入 Private，驱动释放后也不一定立刻还给 OS。

做法：两个配置轮转交错各跑 REP 次，每次只取 10~30s 的稳态窗口（避开启动瞬态），
报告中位数与最小值。配置差异只有一个：frost 是否保持 1/2 分辨率。
"""
import ctypes
import io
import os
import statistics
import subprocess
import sys
import time
from ctypes import wintypes

k = ctypes.WinDLL("kernel32")
ps = ctypes.WinDLL("psapi")
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# 被测可执行文件 = dev playground 产物；可用 VERRE_EXE 覆盖（例如指向旧基线做 A/B）。
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")
REP = int(os.environ.get("AB_REP", "3"))
HOLD = float(os.environ.get("AB_HOLD", "30"))
START = float(os.environ.get("AB_START", "10"))


class PMC(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("pf", wintypes.DWORD),
                ("a", ctypes.c_size_t), ("ws", ctypes.c_size_t),
                ("b", ctypes.c_size_t), ("c", ctypes.c_size_t),
                ("d", ctypes.c_size_t), ("e", ctypes.c_size_t),
                ("f", ctypes.c_size_t), ("g", ctypes.c_size_t),
                ("priv", ctypes.c_size_t)]


def priv_mb(h):
    m = PMC()
    m.cb = ctypes.sizeof(m)
    if ps.GetProcessMemoryInfo(h, ctypes.byref(m), m.cb):
        return m.priv / 1048576.0
    return float("nan")


def one(tag, half):
    env = dict(os.environ)
    # # 前台若恰好是「无边框最大化的普通应用」（Electron / Tauri 那一类），库的独占全屏
    # # 检测（方案 R-16）会命中 -> 画布隐藏 + 整屏停产 -> 本脚本拿到 0 帧连拍并假 FAIL。
    # # 取证必须对「用户前台开着哪个应用」保持 hermetic，与 GLASS_ADAPTIVE=0 同理。
    env["GLASS_FS_HIDE"] = "0"
    # 库默认**不**自建玻璃窗口（上层应用负责 create_window）；
    # 取证脚本需要一块确定性的默认玻璃，因此显式打开。
    env["GLASS_DEFAULT_BLOCK"] = "1"
    env["GLASS_MANY"] = "1"
    env["GLASS_METRICS"] = "1"
    env["GLASS_ADAPTIVE"] = "0"      # 固定 level 0，排除控制器干扰
    if half:
        env["GLASS_FROST_HALF"] = "1"
    else:
        env.pop("GLASS_FROST_HALF", None)
    log = io.open(os.path.join(ROOT, "tools", "abmem_%s.log" % tag), "wb")
    p = subprocess.Popen([EXE], env=env, cwd=ROOT, stdout=subprocess.DEVNULL, stderr=log)
    h = k.OpenProcess(0x0410, False, p.pid)
    t0 = time.perf_counter()
    vals = []
    while time.perf_counter() - t0 < HOLD:
        t = time.perf_counter() - t0
        if t >= START:
            v = priv_mb(h)
            if v == v and v > 1:
                vals.append(v)
        time.sleep(0.4)
    p.terminate()
    try:
        p.wait(timeout=8)
    except Exception:
        p.kill()
    log.close()
    k.CloseHandle(h)
    return vals


res = {"full": [], "half": []}
for rep in range(REP):
    for tag, half in (("full", False), ("half", True)):
        v = one(tag, half)
        med = statistics.median(v) if v else float("nan")
        res[tag].append(med)
        print("rep%d %-4s n=%2d 中位=%.1f MB  最小=%.1f  最大=%.1f"
              % (rep, tag, len(v), med, min(v), max(v)))
        sys.stdout.flush()
        time.sleep(2.0)

print("")
fm = statistics.median(res["full"])
hm = statistics.median(res["half"])
print("=== 汇总（%d 次重复的中位数的中位数）===" % REP)
print("  frost 全分辨率（level 0）: %.1f MB   逐次 %s" % (fm, ["%.1f" % x for x in res["full"]]))
print("  frost 半分辨率（level 1）: %.1f MB   逐次 %s" % (hm, ["%.1f" % x for x in res["half"]]))
print("  差值: %+.1f MB" % (hm - fm))
