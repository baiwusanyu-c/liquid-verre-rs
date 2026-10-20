# -*- coding: utf-8 -*-
"""gate_perf.py -- 性能阀门：一条命令跑关键性能指标并按阈值判 PASS/FAIL。

阈值（与 BASELINE.md / performance_report.md 第十三、十四轮一致）：
  硬门槛（FAIL 即退出码 1）：
    single-drag  fps >= 90  且 p50 <= 9.0 ms      （120Hz 屏单块必须锁相）
    many-drag    fps >= 60                        （方案 ④ 的下限）
    many-drag + SIM_BACKDROP(900x700@120) + lat=2  fps >= 90
        （确定性负载下测"我们自己的吞吐"；真实桌面的 fps 受桌面重绘率/DWM 背压
          支配，84~105 波动，只作软门槛）
    idle Private < 200 MB                         （方案 ③）
  软门槛（只 WARN）：
    many-drag 默认档 fps >= 90（受桌面重绘负载/DWM 背压影响，非渲染成本）
    拖拽 CPU < 5% 单核（同上，安静桌面才稳定达标）

每次运行都会打印 dda_acq / dirty，便于在 FAIL 时区分「我们慢」还是「桌面/DWM 忙」。
"""
import json
import os
import statistics
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROBE = os.path.join(ROOT, "tools", "perf_probe.py")


def run(case, extra=None, seconds=10.0):
    env = dict(os.environ)
    env.pop("GLASS_FRAME_LATENCY", None)
    if extra:
        env.update(extra)
    out = os.path.join(ROOT, "tools", "gate_%s.json" % case.replace("-", "_"))
    subprocess.run([sys.executable, PROBE, "--case", case, "--seconds", str(seconds),
                    "--warmup", "3", "--out", out], env=env, cwd=ROOT,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    d = json.load(open(out, encoding="utf-8"))[0]
    os.remove(out)
    ms = d["drag"]["metrics"]
    return {
        "fps": statistics.median(m["fps"] for m in ms),
        "p50": statistics.median(m["p50_loop"] for m in ms),
        "p95": statistics.median(m["p95_loop"] for m in ms),
        "cpu": d["drag"]["cpu_pct_of_one_core"],
        "priv": d["idle"]["private_mb"],
        "dda": statistics.median(m["dda_acq"] for m in ms),
        "dirty": statistics.median(m["dirty_mpx_avg"] for m in ms),
        "own": statistics.median(m["stage_us"][0] + m["stage_us"][1] + m["stage_us"][3]
                                 for m in ms),
    }


def main():
    rows = []
    rows.append(("single-drag (lat=默认1)", run("single-drag")))
    rows.append(("many-drag  (lat=默认1)", run("many-drag")))
    rows.append(("many-drag  (lat=2, 合成120Hz负载)",
                 run("many-drag", {"GLASS_FRAME_LATENCY": "2",
                                   "GLASS_SIM_BACKDROP": "900x700@120"})))

    hard = []
    warn = []
    s0, m1, m2 = rows[0][1], rows[1][1], rows[2][1]
    if not (s0["fps"] >= 90 and s0["p50"] <= 9.0):
        hard.append("单块拖拽 fps/p50 未达标: %.1f fps / %.2f ms" % (s0["fps"], s0["p50"]))
    if m1["fps"] < 60:
        hard.append("20 块默认档 fps < 60: %.1f" % m1["fps"])
    if m2["fps"] < 90:
        hard.append("20 块吞吐档(lat=2, 合成负载) fps < 90: %.1f" % m2["fps"])
    if max(r["priv"] for r in (s0, m1, m2)) >= 200:
        hard.append("Private >= 200 MB: %s" % [round(r["priv"], 1) for r in (s0, m1, m2)])
    if m1["fps"] < 90:
        warn.append("20 块默认档 fps < 90（%.1f）：桌面重绘 %0.f/s、dirty %.2f Mpx 时受触发节拍/DWM 背压限制"
                    % (m1["fps"], m1["dda"], m1["dirty"]))
    if m1["cpu"] >= 5 or s0["cpu"] >= 5:
        warn.append("拖拽 CPU >= 5%% 单核（single %.2f%% / many %.2f%%）：高重绘桌面窗口才会超"
                    % (s0["cpu"], m1["cpu"]))

    print("%-26s %7s %7s %7s %7s %8s %7s %8s" %
          ("场景", "fps", "p50ms", "p95ms", "cpu%", "privMB", "dda/s", "own_us"))
    for name, r in rows:
        print("%-26s %7.1f %7.2f %7.2f %7.2f %8.1f %7.0f %8.0f" %
              (name, r["fps"], r["p50"], r["p95"], r["cpu"], r["priv"], r["dda"], r["own"]))
    print("")
    for w in warn:
        print("WARN:", w)
    for h in hard:
        print("FAIL:", h)
    print("")
    print("结论:", "PASS 性能阀门通过" if not hard else "FAIL 性能阀门未通过")
    return 0 if not hard else 1


if __name__ == "__main__":
    sys.exit(main())
