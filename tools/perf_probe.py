"""perf_probe.py -- 液态玻璃性能实测探针（《折射渲染方案》7.6 口径）。

统一测量四件事：
  1) CPU：单核当量占用（Δ进程CPU时间 / Δ墙钟），拖拽段与静止段分开统计；
  2) 内存：常驻 WorkingSet64 与 PrivateUsage（方案要求 RSS < 40 MB）；
  3) 显存：GPU Process Memory 的 Dedicated Usage（方案要求 20 块 ≤ 200 MB）；
  4) 帧节奏：解析被测程序 GLASS_METRICS 的 avg/max render 与 max loop 长尾。

用法：
  python tools/perf_probe.py --case single-idle
  python tools/perf_probe.py --case single-drag
  python tools/perf_probe.py --case many-idle
  python tools/perf_probe.py --case many-drag
"""
from __future__ import annotations

import argparse
import ctypes
import json
import math
import os
import re
import subprocess
import sys
import threading
import time
from ctypes import wintypes

# 探针必须与被测程序同为 Per-Monitor V2，否则 SetCursorPos/GetSystemMetrics
# 会被 DPI 虚拟化，落点偏移到物理屏之外。
try:
    ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
except Exception:
    try:
        ctypes.windll.shcore.SetProcessDpiAwareness(2)
    except Exception:
        ctypes.windll.user32.SetProcessDPIAware()

user32 = ctypes.windll.user32
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
psapi = ctypes.WinDLL("psapi", use_last_error=True)

SM_CXSCREEN = 0
SM_CYSCREEN = 1
PROCESS_QUERY_INFORMATION = 0x0400
PROCESS_VM_READ = 0x0010
MOUSEEVENTF_LEFTDOWN = 0x0002
MOUSEEVENTF_LEFTUP = 0x0004

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# 被测可执行文件 = dev playground 产物；可用 VERRE_EXE 覆盖（例如指向旧基线做 A/B）。
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")


class PROCESS_MEMORY_COUNTERS_EX(ctypes.Structure):
    _fields_ = [
        ("cb", wintypes.DWORD),
        ("PageFaultCount", wintypes.DWORD),
        ("PeakWorkingSetSize", ctypes.c_size_t),
        ("WorkingSetSize", ctypes.c_size_t),
        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
        ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
        ("PagefileUsage", ctypes.c_size_t),
        ("PeakPagefileUsage", ctypes.c_size_t),
        ("PrivateUsage", ctypes.c_size_t),
    ]


def open_process(pid: int):
    h = kernel32.OpenProcess(
        PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, False, pid)
    if not h:
        raise OSError(ctypes.get_last_error())
    return h


def cpu_seconds(handle) -> float:
    creation = wintypes.FILETIME()
    exit_t = wintypes.FILETIME()
    kernel = wintypes.FILETIME()
    user = wintypes.FILETIME()
    if not kernel32.GetProcessTimes(handle, ctypes.byref(creation),
                                    ctypes.byref(exit_t),
                                    ctypes.byref(kernel), ctypes.byref(user)):
        return 0.0
    k = (kernel.dwHighDateTime << 32) | kernel.dwLowDateTime
    u = (user.dwHighDateTime << 32) | user.dwLowDateTime
    return (k + u) / 1e7


def memory_bytes(handle):
    counters = PROCESS_MEMORY_COUNTERS_EX()
    counters.cb = ctypes.sizeof(counters)
    if not psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters),
                                      ctypes.sizeof(counters)):
        return 0, 0
    return counters.WorkingSetSize, counters.PrivateUsage


def gpu_dedicated_mb(pid: int) -> float:
    """读 GPU Process Memory 的 Dedicated + Shared Usage（所有 LUID 求和）。

    本机是 Intel Arc 集显，显存走系统内存，Dedicated Usage 恒为 0，
    真正的占用记在 Shared Usage；两者相加才是该进程的 GPU 侧占用。
    """
    script = (
        "$ErrorActionPreference='SilentlyContinue';$t=0;"
        "foreach($n in 'Dedicated Usage','Shared Usage'){"
        "$c=Get-Counter -Counter ('\\GPU Process Memory(pid_%d_*)\\'+$n);"
        "$t+=($c.CounterSamples | Measure-Object -Property CookedValue -Sum).Sum};"
        "$t" % pid
    )
    try:
        out = subprocess.run(
            ["powershell", "-NoProfile", "-Command", script],
            capture_output=True, text=True, timeout=40)
        txt = out.stdout.strip()
        if not txt:
            return float("nan")
        return float(txt) / (1024 * 1024)
    except Exception:
        return float("nan")


def _f(line, key, cast=float):
    m = re.search(re.escape(key) + r"=(-?[\d.]+)", line)
    return cast(m.group(1)) if m else None


def parse_metrics(line):
    """逐字段宽松解析 [metrics] 行，字段增删不会让探针失效。"""
    if "[metrics]" not in line:
        return None
    d = {}
    for key in ("win_s", "screens", "blocks", "loops", "frames", "fps", "idle",
                "avg_render_ms", "max_render_ms", "max_loop_ms",
                "dirty_mpx_avg", "dirty_mpx_max", "blocks_avg",
                "pass_a_mpx_avg", "dda_acq", "dda_changed", "copy_mpx_avg",
                "cap_max_ms"):
        v = _f(line, key)
        if v is not None:
            d[key] = v
    for key in ("screens", "blocks", "loops", "frames", "idle"):
        if key in d:
            d[key] = int(d[key])
    m = re.search(r"loop_ms_avg=msg([\d.]+)/cap([\d.]+)/prep([\d.]+)", line)
    if m:
        d["loop_ms_msg"] = float(m.group(1))
        d["loop_ms_cap"] = float(m.group(2))
        d["loop_ms_prep"] = float(m.group(3))
    m = re.search(r"stage_us=A(\d+) B(\d+) P(\d+) C(\d+)", line)
    if m:
        d["stage_us"] = [int(x) for x in m.groups()]
    m = re.search(r"cap_split_ms=acq([\d.]+)/dirtyApi([\d.]+)/copy([\d.]+)"
                  r"/pre([\d.]+)/rel([\d.]+)", line)
    if m:
        d["cap_ms_acq"] = float(m.group(1))
        d["cap_ms_dirtyapi"] = float(m.group(2))
        d["cap_ms_copy"] = float(m.group(3))
        d["cap_ms_pre"] = float(m.group(4))
        d["cap_ms_rel"] = float(m.group(5))
    m = re.search(r"p50_ms=loop([\d.]+)/cap([\d.]+)/ren([\d.]+)", line)
    if m:
        d["p50_loop"] = float(m.group(1))
        d["p50_cap"] = float(m.group(2))
        d["p50_ren"] = float(m.group(3))
    m = re.search(r"p95_ms=loop([\d.]+)/cap([\d.]+)/ren([\d.]+)", line)
    if m:
        d["p95_loop"] = float(m.group(1))
        d["p95_cap"] = float(m.group(2))
        d["p95_ren"] = float(m.group(3))
    for k in ("acq_max_ms", "rel_max_ms"):
        v = _f(line, k)
        if v is not None:
            d[k] = v
    m = re.search(r"pass_a=([\d.]+)/frame", line)
    if m:
        d["pass_a_per_frame"] = float(m.group(1))
    m = re.search(r"pass_a_throttled=([\d.]+)/frame", line)
    if m:
        d["pass_a_throttled"] = float(m.group(1))
    return d or None


class LogReader(threading.Thread):
    def __init__(self, stream):
        super().__init__(daemon=True)
        self.stream = stream
        self.lines: list[str] = []
        self.metrics: list[dict] = []

    def run(self):
        for raw in self.stream:
            line = raw.decode("utf-8", "replace").rstrip()
            if not line:
                continue
            self.lines.append(line)
            d = parse_metrics(line)
            if d:
                self.metrics.append(d)
            if False:
                self.metrics.append({
                    "screens": int(d["screens"]),
                    "blocks": int(d["blocks"]),
                    "frames": int(d["frames"]),
                    "fps": float(d["fps"]),
                    "idle_loops": int(d["idle"]),
                    "avg_render_ms": float(d["avg"]),
                    "max_render_ms": float(d["mr"]),
                    "max_loop_ms": float(d["ml"]),
                    "stage_us": [int(d["a"]), int(d["b"]),
                                 int(d["p"]), int(d["c"])],
                    "dirty_mpx_avg": float(d["davg"]),
                    "dirty_mpx_max": float(d["dmax"]),
                    "blocks_drawn_avg": float(d["bavg"]),
                    "pass_a_per_frame": float(d["pa"]),
                    "pass_a_mpx_avg": float(d["pam"]),
                    "dda_acq_per_s": float(d["acq"]),
                    "dda_changed_per_s": float(d["chg"]),
                    "copy_mpx_avg": float(d["cp"]),
                })

    def metrics_since(self, index: int):
        return self.metrics[index:]


def set_cursor(x: float, y: float):
    user32.SetCursorPos(int(round(x)), int(round(y)))


def screen_size():
    return (user32.GetSystemMetrics(SM_CXSCREEN),
            user32.GetSystemMetrics(SM_CYSCREEN))


def drag_path(t: float, cx: float, cy: float, span: float):
    """模拟人工手速：正弦横扫 + 轻微纵向漂移，峰值速度约 1.3 k px/s。"""
    x = cx + span * 0.5 * math.sin(2.0 * math.pi * 0.55 * t)
    y = cy + span * 0.18 * math.sin(2.0 * math.pi * 0.31 * t + 0.7)
    return x, y


def run_case(case: str, seconds: float, warmup: float) -> dict:
    many = case.startswith("many")
    transparent = case.startswith("transparent")
    do_drag = case.endswith("drag")
    env = dict(os.environ)
    # # 前台若恰好是「无边框最大化的普通应用」（Electron / Tauri 那一类），库的独占全屏
    # # 检测（方案 R-16）会命中 -> 画布隐藏 + 整屏停产 -> 本脚本拿到 0 帧连拍并假 FAIL。
    # # 取证必须对「用户前台开着哪个应用」保持 hermetic，与 GLASS_ADAPTIVE=0 同理。
    env["GLASS_FS_HIDE"] = "0"
    # 库默认**不**自建玻璃窗口（上层应用负责 create_window）；
    # 取证脚本需要一块确定性的默认玻璃，因此显式打开。
    env["GLASS_DEFAULT_BLOCK"] = "1"
    env["GLASS_METRICS"] = "1"
    if many:
        env["GLASS_MANY"] = "1"
    env.pop("GLASS_EXTREME_SPEC", None)

    # 透明玻璃场景（需求 F3.1 / 验收 A9）：用 playground 的 --transparent 把
    # 引擎默认样式切成透明模式，--no-demo-pair 保证场景里只有一块玻璃，
    # 与 single-* 场景的几何完全可比。
    argv = [EXE, "--no-browser"] + (["--transparent", "--no-demo-pair"] if transparent else [])
    # ⚠ 必须 --no-browser：panel.rs 在不带它时会 `cmd /c start` 打开浏览器，而 playground 被
# terminate 后标签页不会关 => 每次采样累积一个页面，CPU/内存被浏览器吃掉、浏览器还抢前台（前台一被交还就可能落到 `Progman` 上，连带触发独占全屏检测）。
    proc = subprocess.Popen(argv, env=env, cwd=ROOT,
                            stdout=subprocess.DEVNULL,
                            stderr=subprocess.PIPE)
    reader = LogReader(proc.stderr)
    reader.start()
    handle = open_process(proc.pid)

    sw, sh = screen_size()
    # 块 0 在 GLASS_MANY 布局下固定为 (32,64) 600x400；单块 / 透明单块居中。
    if many:
        cx, cy = 32.0 + 300.0, 64.0 + 200.0
    else:
        cx, cy = sw * 0.5, sh * 0.5

    set_cursor(cx, cy)
    time.sleep(warmup * 0.6)
    # GPU 显存计数器的采集会 spawn PowerShell 并短暂拖慢整机，
    # 必须放在测量窗口之外，否则会污染 max_loop 长尾。
    gpu_mb = gpu_dedicated_mb(proc.pid) if os.environ.get("PROBE_GPU_MB") == "1" else float("nan")
    time.sleep(warmup * 0.4)

    result = {"case": case, "pid": proc.pid, "screen": [sw, sh],
              "warmup_s": warmup, "measure_s": seconds, "drag": do_drag}

    # ---- 静止段 ----
    base_marks = len(reader.metrics)
    t0, c0 = time.perf_counter(), cpu_seconds(handle)
    ws_samples, priv_samples = [], []
    end = t0 + (seconds if not do_drag else seconds * 0.4)
    while time.perf_counter() < end:
        ws, priv = memory_bytes(handle)
        ws_samples.append(ws)
        priv_samples.append(priv)
        time.sleep(0.15)
    t1, c1 = time.perf_counter(), cpu_seconds(handle)
    result["idle"] = {
        "wall_s": round(t1 - t0, 3),
        "cpu_pct_of_one_core": round((c1 - c0) / max(t1 - t0, 1e-6) * 100, 2),
        "working_set_mb": round(max(ws_samples) / 1048576, 1) if ws_samples else 0,
        "working_set_avg_mb": round(
            (sum(ws_samples) / len(ws_samples)) / 1048576, 1) if ws_samples else 0,
        "private_mb": round(max(priv_samples) / 1048576, 1) if priv_samples else 0,
        "gpu_dedicated_mb": round(gpu_mb, 1),
        "metrics": reader.metrics_since(base_marks),
    }

    # ---- 拖拽段 ----
    if do_drag:
        set_cursor(cx, cy)
        time.sleep(0.25)
        user32.mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0)
        time.sleep(0.06)
        base_marks = len(reader.metrics)
        t0, c0 = time.perf_counter(), cpu_seconds(handle)
        span = min(520.0, sw * 0.4)
        last = t0
        max_loop = 0.0
        try:
            while True:
                now = time.perf_counter()
                if now - t0 > seconds:
                    break
                x, y = drag_path(now - t0, cx, cy, span)
                set_cursor(x, y)
                max_loop = max(max_loop, time.perf_counter() - last)
                last = time.perf_counter()
                # 250 Hz 采样，接近 1000 Hz 鼠标的合并前速率
                time.sleep(max(0.0, 0.004 - (time.perf_counter() - now)))
        finally:
            user32.mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0)
        t1, c1 = time.perf_counter(), cpu_seconds(handle)
        result["drag"] = {
            "wall_s": round(t1 - t0, 3),
            "cpu_pct_of_one_core": round(
                (c1 - c0) / max(t1 - t0, 1e-6) * 100, 2),
            "probe_max_step_ms": round(max_loop * 1000, 2),
            "metrics": reader.metrics_since(base_marks),
        }
        time.sleep(0.4)

    proc.terminate()
    try:
        proc.wait(timeout=8)
    except subprocess.TimeoutExpired:
        proc.kill()
    kernel32.CloseHandle(handle)
    result["stderr_tail"] = reader.lines[-6:]
    result["warnings"] = [l for l in reader.lines
                          if "[warn]" in l or "[info]" in l or "DDA" in l][:12]
    logpath = os.path.join(ROOT, "tools", "probe_%s.log" % case)
    with open(logpath, "w", encoding="utf-8") as fh:
        fh.write("\n".join(reader.lines))
    return result


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--case", default="single-idle",
                    choices=["single-idle", "single-drag",
                             "many-idle", "many-drag",
                             "transparent-idle", "transparent-drag", "all"])
    ap.add_argument("--seconds", type=float, default=12.0)
    ap.add_argument("--warmup", type=float, default=3.0)
    ap.add_argument("--out", default="")
    args = ap.parse_args()

    cases = (["single-idle", "single-drag", "many-idle", "many-drag"]
             if args.case == "all" else [args.case])
    payload = []
    for case in cases:
        if not os.path.exists(EXE):
            print("缺少 release 可执行文件：%s" % EXE)
            return 2
        payload.append(run_case(case, args.seconds, args.warmup))
        print(json.dumps(payload[-1], ensure_ascii=False, indent=1))
    if args.out:
        with open(args.out, "w", encoding="utf-8") as fh:
            json.dump(payload, fh, ensure_ascii=False, indent=1)
    return 0


if __name__ == "__main__":
    sys.exit(main())
