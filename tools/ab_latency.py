# -*- coding: utf-8 -*-
"""ab_latency.py -- GLASS_FRAME_LATENCY 1 vs 2 的轮转交错 A/B（many/single 拖拽）。"""
import os
import statistics
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import importlib.util
spec = importlib.util.spec_from_file_location(
    "pp", os.path.join(os.path.dirname(os.path.abspath(__file__)), "perf_probe.py"))
pp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pp)

REP = int(os.environ.get("AB_REP", "3"))
res = {}
for rep in range(REP):
    for case in ("many-drag", "single-drag"):
        for lat in ("1", "2"):
            os.environ["GLASS_FRAME_LATENCY"] = lat
            r = pp.run_case(case, 10.0, 3.0)
            ms = (r.get("drag") or {}).get("metrics") or []
            row = {
                "fps": statistics.median([m["fps"] for m in ms]),
                "p50": statistics.median([m["p50_loop"] for m in ms]),
                "p95": statistics.median([m["p95_loop"] for m in ms]),
                "maxloop": max([m["max_loop_ms"] for m in ms], default=0),
                "cpu": (r.get("drag") or {}).get("cpu_pct_of_one_core", float("nan")),
            }
            res.setdefault((case, lat), []).append(row)
            print("rep%d %-11s lat=%s fps=%6.1f p50=%6.2f p95=%6.2f max=%6.1f cpu=%5.2f"
                  % (rep, case, lat, row["fps"], row["p50"], row["p95"], row["maxloop"], row["cpu"]),
                  flush=True)
os.environ.pop("GLASS_FRAME_LATENCY", None)
print("")
print("=== 中位数汇总 ===")
for case in ("many-drag", "single-drag"):
    for k in ("fps", "p50", "p95", "maxloop", "cpu"):
        v1 = statistics.median([r[k] for r in res[(case, "1")]])
        v2 = statistics.median([r[k] for r in res[(case, "2")]])
        print("  %-11s %-7s lat1=%8.2f  lat2=%8.2f  变化=%+6.1f%%"
              % (case, k, v1, v2, (v2 - v1) / v1 * 100.0 if v1 else 0.0))
