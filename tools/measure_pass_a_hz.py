"""measure_pass_a_hz.py -- Pass A 限频的判定实验。

问题：20 块拖拽在真实桌面下只有 79.2 fps、p50 单循环 12.90 ms（介于 8.33 与
16.67 之间 = 帧长抖动 judder）。Pass A 限频能否把它锁回 8.33 ms？

按 round-robin 交错跑 HZ_LIST × REPS 次，报中位数与极差。
"""
import io
import json
import os
import statistics
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "tools"))
import importlib.util

spec = importlib.util.spec_from_file_location(
    "pp", os.path.join(ROOT, "tools", "perf_probe.py"))
pp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pp)

REPS = int(os.environ.get("HZ_REPS", "3"))
SECONDS = float(os.environ.get("HZ_SECONDS", "10"))
CASE = os.environ.get("HZ_CASE", "many-drag")
HZ_LIST = [x.strip() for x in
           os.environ.get("HZ_LIST", "0,60,90").split(",") if x.strip()]
EXTRA = {}
for k in ("HZ_MONITORS", "HZ_SIM", "HZ_MANY"):
    if os.environ.get(k):
        EXTRA[{"HZ_MONITORS": "GLASS_MONITORS", "HZ_SIM": "GLASS_SIM_BACKDROP",
               "HZ_MANY": "GLASS_MANY"}[k]] = os.environ[k]


def avg(ms, k):
    v = [m[k] for m in ms if k in m]
    return sum(v) / len(v) if v else float("nan")


def med(ms, k):
    v = sorted(x[k] for x in ms if k in x)
    return v[len(v) // 2] if v else float("nan")


def mx(ms, k):
    v = [m[k] for m in ms if k in m]
    return max(v) if v else float("nan")


def one(hz):
    env = {"GLASS_PASS_A_HZ": hz}
    env.update(EXTRA)
    old = {k: os.environ.get(k) for k in env}
    os.environ.update(env)
    try:
        r = pp.run_case(CASE, SECONDS, 3.0)
    finally:
        for k, v in old.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
    ph = r.get("drag") if isinstance(r.get("drag"), dict) else r.get("idle")
    ms = (ph or {}).get("metrics") or []
    return {
        "hz": hz,
        "fps": avg(ms, "fps"),
        "cpu": (ph or {}).get("cpu_pct_of_one_core"),
        "p50_loop": med(ms, "p50_loop"),
        "p95_loop": med(ms, "p95_loop"),
        "maxloop": mx(ms, "max_loop_ms"),
        "p50_cap": med(ms, "p50_cap"),
        "p50_ren": med(ms, "p50_ren"),
        "passA": avg(ms, "pass_a_mpx_avg"),
        "passA_runs": avg(ms, "pass_a_per_frame"),
        "throttled": avg(ms, "pass_a_throttled"),
        "dirty": avg(ms, "dirty_mpx_avg"),
        "dda": avg(ms, "dda_changed"),
        "ws": (r.get("idle") or {}).get("working_set_mb"),
        "priv": (r.get("idle") or {}).get("private_mb"),
    }


results = {hz: [] for hz in HZ_LIST}
for rep in range(REPS):
    for hz in HZ_LIST:
        d = one(hz)
        results[hz].append(d)
        print("rep%d hz=%-4s fps=%6.1f cpu=%5.2f%% p50loop=%6.2f p95loop=%6.2f "
              "maxloop=%6.1f p50ren=%5.2f | passA跑=%.2f/帧 限频跳过=%.2f/帧 "
              "passAMpx=%.3f dirty=%.3f ddaChg=%.1f"
              % (rep, hz, d["fps"], d["cpu"] or float("nan"), d["p50_loop"],
                 d["p95_loop"], d["maxloop"], d["p50_ren"], d["passA_runs"],
                 d["throttled"], d["passA"], d["dirty"], d["dda"]), flush=True)


def md(vs):
    vs = [v for v in vs if v == v]
    return statistics.median(vs) if vs else float("nan")


print("")
print("case=%s  reps=%d" % (CASE, REPS))
print("%-6s %8s %8s %8s | %8s %8s %8s | %8s %8s %8s %8s" % (
    "hz", "fps中位", "fps最小", "fps最大",
    "p50loop", "p95loop", "maxloop", "passA跑/帧", "限频/帧", "passAMpx", "cpu%"))
summary = []
for hz in HZ_LIST:
    rs = results[hz]
    fps = [r["fps"] for r in rs]
    row = {"hz": hz, "fps_all": fps, "fps_med": md(fps),
           "fps_min": min(fps), "fps_max": max(fps)}
    for k in ("p50_loop", "p95_loop", "maxloop", "passA_runs", "throttled",
              "passA", "cpu", "dirty", "dda", "p50_ren"):
        row[k] = md([r[k] for r in rs if r[k] is not None])
    summary.append(row)
    print("%-6s %8.1f %8.1f %8.1f | %8.2f %8.2f %8.1f | %8.2f %8.2f %8.3f %8.2f" % (
        hz, row["fps_med"], row["fps_min"], row["fps_max"],
        row["p50_loop"], row["p95_loop"], row["maxloop"],
        row["passA_runs"], row["throttled"], row["passA"], row["cpu"]))

base = next((r for r in summary if r["hz"] == "0"), None)
if base:
    print("")
    print("相对不限频(hz=0, fps %.1f, p50loop %.2f ms)：" % (base["fps_med"], base["p50_loop"]))
    for r in summary:
        if r["hz"] != "0":
            print("  hz=%-4s fps %+6.1f (%+.0f%%)  p50loop %+6.2f ms  p95loop %+6.2f ms  "
                  "maxloop %+7.1f ms  cpu %+5.2f%%  [fps 各次: %s]" % (
                      r["hz"], r["fps_med"] - base["fps_med"],
                      100.0 * (r["fps_med"] - base["fps_med"]) / max(base["fps_med"], .1),
                      r["p50_loop"] - base["p50_loop"],
                      r["p95_loop"] - base["p95_loop"],
                      r["maxloop"] - base["maxloop"],
                      r["cpu"] - base["cpu"],
                      ", ".join("%.1f" % x for x in r["fps_all"])))

json.dump({"summary": summary, "raw": results},
          io.open(os.path.join(ROOT, "tools", "pass_a_hz.json"), "w",
                  encoding="utf-8"), ensure_ascii=False, indent=1)
