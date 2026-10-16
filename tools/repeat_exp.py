"""repeat_exp.py -- 轮转重复实验，用中位数和极差把"噪声"与"效应"分开。

配置按 round-robin 交错执行（A B C A B C ...），这样机器的慢漂移会均摊到
所有配置上，不会系统性地偏袒某一个。每个配置跑 REPS 次，报告中位数与极差。
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

REPS = int(os.environ.get("EXP_REPS", "3"))
SECONDS = float(os.environ.get("EXP_SECONDS", "6"))
CASE = os.environ.get("EXP_CASE", "many-drag")

COMMON = {"GLASS_MANY": "1", "GLASS_MONITORS": "1",
          "GLASS_SIM_BACKDROP": "900x700@120",
          "GLASS_SIM_DIRTY": "300x220@12"}

CONFIGS = [
    ("baseline", {}),
    ("copy=union", {"GLASS_COPY_MODE": "union"}),
    ("copy=full", {"GLASS_COPY_MODE": "full"}),
    ("skip-copy", {"GLASS_SKIP_COPY": "1"}),
    ("skip-PassA", {"GLASS_SKIP_COPY": "1", "GLASS_SKIP_PASS_A": "1"}),
    ("skip-Present", {"GLASS_SKIP_COPY": "1", "GLASS_SKIP_PRESENT": "1"}),
]


def avg(ms, k):
    v = [m[k] for m in ms if k in m]
    return sum(v) / len(v) if v else float("nan")


def mx(ms, k):
    v = [m[k] for m in ms if k in m]
    return max(v) if v else float("nan")


def one(extra):
    keys = list(COMMON) + list(extra)
    old = {k: os.environ.get(k) for k in keys}
    os.environ.update(COMMON)
    os.environ.update(extra)
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
        "fps": avg(ms, "fps"),
        "cpu": (ph or {}).get("cpu_pct_of_one_core"),
        # 分位数：对离群值稳健（均值在每窗口只有几十次采样时会被单个
        # 几百毫秒的离群值彻底带偏）
        "p50_loop": avg(ms, "p50_loop"),
        "p95_loop": avg(ms, "p95_loop"),
        "p50_cap": avg(ms, "p50_cap"),
        "p95_cap": avg(ms, "p95_cap"),
        "p50_ren": avg(ms, "p50_ren"),
        "p95_ren": avg(ms, "p95_ren"),
        "acq": avg(ms, "cap_ms_acq"),
        "dirtyapi": avg(ms, "cap_ms_dirtyapi"),
        "copy": avg(ms, "cap_ms_copy"),
        "pre": avg(ms, "cap_ms_pre"),
        "rel": avg(ms, "cap_ms_rel"),
        "acq_max": mx(ms, "acq_max_ms"),
        "rel_max": mx(ms, "rel_max_ms"),
        "render": avg(ms, "avg_render_ms"),
        "maxloop": mx(ms, "max_loop_ms"),
        "copyMpx": avg(ms, "copy_mpx_avg"),
        "ddaAcq": avg(ms, "dda_acq"),
        "loops": avg(ms, "loops"),
    }


results = {tag: [] for tag, _ in CONFIGS}
for rep in range(REPS):
    for tag, extra in CONFIGS:
        d = one(extra)
        results[tag].append(d)
        print("rep%d %-13s fps=%5.1f loops=%4.0f | p50 loop=%6.2f cap=%6.2f ren=%6.2f "
              "| p95 loop=%7.2f | cap细分 acq=%6.2f dirtyApi=%5.2f copy=%5.2f pre=%6.2f "
              "rel=%6.2f | acqMax=%7.1f relMax=%7.1f | copyMpx=%5.2f ddaAcq=%5.1f"
              % (rep, tag, d["fps"], d["loops"], d["p50_loop"], d["p50_cap"],
                 d["p50_ren"], d["p95_loop"], d["acq"], d["dirtyapi"], d["copy"],
                 d["pre"], d["rel"], d["acq_max"], d["rel_max"], d["copyMpx"],
                 d["ddaAcq"]), flush=True)


def med(vs):
    vs = [v for v in vs if v == v]
    return statistics.median(vs) if vs else float("nan")


print("")
print("%-13s %7s %7s %7s | %8s %8s %8s | %8s %8s %8s | %6s %6s %6s %6s %6s" % (
    "config", "fps中位", "fps最小", "fps最大",
    "p50loop", "p50cap", "p50ren", "p95loop", "p95cap", "p95ren",
    "acq", "pre", "rel", "copy", "copyMpx"))
summary = []
for tag, _ in CONFIGS:
    rs = results[tag]
    fps = [r["fps"] for r in rs]
    row = {"tag": tag, "fps_all": fps,
           "fps_med": med(fps), "fps_min": min(fps), "fps_max": max(fps)}
    for k in ("p50_loop", "p50_cap", "p50_ren", "p95_loop", "p95_cap", "p95_ren",
              "acq", "pre", "rel", "copy", "copyMpx", "acq_max", "rel_max", "ddaAcq"):
        row[k + "_med"] = med([r[k] for r in rs])
    summary.append(row)
    print("%-13s %7.1f %7.1f %7.1f | %8.2f %8.2f %8.2f | %8.2f %8.2f %8.2f | "
          "%6.2f %6.2f %6.2f %6.2f %6.2f" % (
              tag, row["fps_med"], row["fps_min"], row["fps_max"],
              row["p50_loop_med"], row["p50_cap_med"], row["p50_ren_med"],
              row["p95_loop_med"], row["p95_cap_med"], row["p95_ren_med"],
              row["acq_med"], row["pre_med"], row["rel_med"], row["copy_med"],
              row["copyMpx_med"]))

base = next((r for r in summary if r["tag"] == "baseline"), None)
if base:
    print("")
    print("相对 baseline（fps 中位 %.1f，p50 loop %.2f ms）："
          % (base["fps_med"], base["p50_loop_med"]))
    for r in summary:
        if r["tag"] != "baseline":
            dloop = r["p50_loop_med"] - base["p50_loop_med"]
            print("  %-13s fps %+6.1f (%+.0f%%)  p50loop %+7.2f ms  p50cap %+7.2f  "
                  "p50ren %+7.2f   [fps 各次: %s]" % (
                      r["tag"], r["fps_med"] - base["fps_med"],
                      100.0 * (r["fps_med"] - base["fps_med"]) / max(base["fps_med"], 0.1),
                      dloop, r["p50_cap_med"] - base["p50_cap_med"],
                      r["p50_ren_med"] - base["p50_ren_med"],
                      ", ".join("%.1f" % x for x in r["fps_all"])))

json.dump({"summary": summary, "raw": results},
          io.open(os.path.join(ROOT, "tools", "repeat_exp.json"), "w",
                  encoding="utf-8"), ensure_ascii=False, indent=1)
