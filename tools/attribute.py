"""attribute.py -- 拖拽帧时间的受控归因实验。

在 many-drag（最差场景）上逐个关掉一段 GPU 工作，看 fps 抬升多少，
从而定位真正的吞吐瓶颈。所有开关默认关闭，只用于实验。
"""
import io
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "tools"))
import importlib.util

spec = importlib.util.spec_from_file_location(
    "pp", os.path.join(ROOT, "tools", "perf_probe.py"))
pp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pp)

CASE = os.environ.get("ATTR_CASE", "many-drag")
MATRIX = [
    ("baseline(2屏)", {}),
    ("only-primary", {"GLASS_MONITORS": "1"}),
    ("skip-PassA", {"GLASS_SKIP_PASS_A": "1"}),
    ("skip-PassB", {"GLASS_SKIP_PASS_B": "1"}),
    ("skip-Present", {"GLASS_SKIP_PRESENT": "1"}),
    ("skip-A+B", {"GLASS_SKIP_PASS_A": "1", "GLASS_SKIP_PASS_B": "1"}),
    ("skip-clear", {"GLASS_SKIP_CLEAR": "1"}),
    ("skip-dirty", {"GLASS_SKIP_DIRTY": "1"}),
    ("skip-copy", {"GLASS_SKIP_COPY": "1"}),
]


def avg(ms, k):
    v = [m[k] for m in ms if k in m]
    return sum(v) / len(v) if v else float("nan")


def mx(ms, k):
    v = [m[k] for m in ms if k in m]
    return max(v) if v else float("nan")


rows = []
for tag, extra in MATRIX:
    old = {k: os.environ.get(k) for k in extra}
    os.environ.update(extra)
    try:
        r = pp.run_case(CASE, 10.0, 3.0)
    finally:
        for k, v in old.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
    d = r.get("drag") or {}
    ms = d.get("metrics") or []
    rows.append({"tag": tag, "env": extra, "drag": d, "idle": r.get("idle")})
    print("%-16s fps=%6.1f  cpu=%5.2f%%  avgRend=%6.3f  cap=%6.3f  maxloop=%7.1f"
          % (tag, avg(ms, "fps"), d.get("cpu_pct_of_one_core", float("nan")),
             avg(ms, "avg_render_ms"), avg(ms, "loop_ms_cap"),
             mx(ms, "max_loop_ms")), flush=True)

json.dump(rows, io.open(os.path.join(ROOT, "tools", "attribute.json"), "w",
                        encoding="utf-8"), ensure_ascii=False, indent=1)
