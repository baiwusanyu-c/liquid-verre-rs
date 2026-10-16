"""experiments.py -- 两个判定实验（确定性负载）。

真实桌面的活动率会在 4~52 次/秒之间漂移，8 秒窗口不可比。这里用
  GLASS_SIM_BACKDROP=900x700@120  固定 Pass A/B 的脏区尺寸与节拍（每帧都渲染）
  GLASS_SIM_DIRTY=300x220@12      固定 DDA 的搬运量（12 个 300x220 散布矩形）
  GLASS_MONITORS=1                排除第二块屏的变量
把负载完全确定性化后再做分段归因。

实验 A：拷贝策略 multi（逐矩形）/ union（并集一次）/ full（整屏）
实验 B：分段跳过 PassA / PassB / Clear / Present，看各自对 fps 的真实贡献
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

COMMON = {
    "GLASS_MANY": "1",
    "GLASS_MONITORS": "1",
    "GLASS_SIM_BACKDROP": "900x700@120",
}
EXP_A = [
    ("A", "copy=multi(现状)", {"GLASS_SIM_DIRTY": "300x220@12", "GLASS_COPY_MODE": "multi"}),
    ("A", "copy=union", {"GLASS_SIM_DIRTY": "300x220@12", "GLASS_COPY_MODE": "union"}),
    ("A", "copy=full", {"GLASS_SIM_DIRTY": "300x220@12", "GLASS_COPY_MODE": "full"}),
    ("A", "copy=跳过", {"GLASS_SKIP_COPY": "1"}),
]
EXP_B = [
    ("B", "baseline", {"GLASS_SKIP_COPY": "1"}),
    ("B", "skip-PassA", {"GLASS_SKIP_COPY": "1", "GLASS_SKIP_PASS_A": "1"}),
    ("B", "skip-PassB", {"GLASS_SKIP_COPY": "1", "GLASS_SKIP_PASS_B": "1"}),
    ("B", "skip-Clear", {"GLASS_SKIP_COPY": "1", "GLASS_SKIP_CLEAR": "1"}),
    ("B", "skip-Present", {"GLASS_SKIP_COPY": "1", "GLASS_SKIP_PRESENT": "1"}),
    ("B", "skip-A+B+Clear", {"GLASS_SKIP_COPY": "1", "GLASS_SKIP_PASS_A": "1",
                             "GLASS_SKIP_PASS_B": "1", "GLASS_SKIP_CLEAR": "1"}),
]


def avg(ms, k):
    v = [m[k] for m in ms if k in m]
    return sum(v) / len(v) if v else float("nan")


def mx(ms, k):
    v = [m[k] for m in ms if k in m]
    return max(v) if v else float("nan")


def run(group, tag, extra, case, seconds):
    keys = list(COMMON) + list(extra)
    old = {k: os.environ.get(k) for k in keys}
    os.environ.update(COMMON)
    os.environ.update(extra)
    try:
        r = pp.run_case(case, seconds, 3.0)
    finally:
        for k, v in old.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
    ph = r.get("drag") if isinstance(r.get("drag"), dict) else r.get("idle")
    ms = (ph or {}).get("metrics") or []
    row = {
        "group": group, "tag": tag, "env": extra, "case": case,
        "fps": avg(ms, "fps"), "cpu": (ph or {}).get("cpu_pct_of_one_core"),
        "cap": avg(ms, "loop_ms_cap"), "cap_max": mx(ms, "cap_max_ms"),
        "render": avg(ms, "avg_render_ms"), "render_max": mx(ms, "max_render_ms"),
        "msg": avg(ms, "loop_ms_msg"), "prep": avg(ms, "loop_ms_prep"),
        "maxloop": mx(ms, "max_loop_ms"),
        "dirty": avg(ms, "dirty_mpx_avg"), "passA": avg(ms, "pass_a_mpx_avg"),
        "copyMpx": avg(ms, "copy_mpx_avg"),
    }
    print("%s %-16s fps=%6.1f cpu=%5.2f%% cap=%6.3f render=%6.3f maxloop=%6.1f "
          "dirty=%.3f passA=%.3f copyMpx=%.3f"
          % (group, tag, row["fps"], row["cpu"] or float("nan"), row["cap"],
             row["render"], row["maxloop"], row["dirty"], row["passA"],
             row["copyMpx"]), flush=True)
    return row


rows = []
print("=== 实验 A：DDA 拷贝策略（20 块，单屏，模拟脏区 900x700@120Hz）===")
for g, t, e in EXP_A:
    rows.append(run(g, t, e, "many-drag", 8.0))
print("")
print("=== 实验 B：分段归因（同样负载，拷贝已跳过以排除该变量）===")
for g, t, e in EXP_B:
    rows.append(run(g, t, e, "many-drag", 8.0))

json.dump(rows, io.open(os.path.join(ROOT, "tools", "experiments.json"), "w",
                        encoding="utf-8"), ensure_ascii=False, indent=1)
base = next((r for r in rows if r["tag"] == "baseline"), None)
if base:
    print("")
    print("相对 baseline(%.1f fps)：" % base["fps"])
    for r in rows:
        if r["group"] == "B" and r["tag"] != "baseline":
            print("  %-16s %+7.1f fps (%+.0f%%)"
                  % (r["tag"], r["fps"] - base["fps"],
                     100.0 * (r["fps"] - base["fps"]) / max(base["fps"], 0.1)))
