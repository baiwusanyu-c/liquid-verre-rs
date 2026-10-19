# -*- coding: utf-8 -*-
"""ab_influence.py -- 同环境 A/B：影响盒 2.5R+32（修复前）vs 3R+32 + gate（修复后）。

两个二进制只差「shader gate + 影响盒半径」，用轮转交错（round-robin）各跑 REP 次，
按中位数比较，排除桌面负载随时间漂移带来的偏差。
"""

先造出两个二进制（仓库里不存 exe）：

    cargo build --release
    Copy-Item target/release/liquid-glass.exe tools/exe_fixed.exe
    git stash push shaders/glass_ps.hlsl src/gfx/compositor.rs   # 回到修复前
    cargo build --release
    Copy-Item target/release/liquid-glass.exe tools/exe_prefix.exe
    git stash pop
    cargo build --release                                        # 恢复工作区二进制

注意 git stash pop 之后必须重新 build：Copy-Item 会保留源文件的旧 mtime，
cargo 会误判为「无需重建」（本轮真踩过：测的其实是上一个二进制）。
"""
import io
import json
import os
import shutil
import statistics
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "release", "liquid-glass.exe")
TAGS = ("prefix", "fixed")
CASES = ("single-drag", "many-drag")
REP = int(os.environ.get("AB_REP", "3"))
SEC = os.environ.get("AB_SECONDS", "8")
WARM = os.environ.get("AB_WARMUP", "3")

res = {}
for rep in range(REP):
    for case in CASES:
        for tag in TAGS:
            shutil.copyfile(os.path.join(ROOT, "tools", "exe_%s.exe" % tag), EXE)
            out = os.path.join(ROOT, "tools", "ab_%s_%s_%d.json" % (case.replace("-", "_"), tag, rep))
            subprocess.run([sys.executable, os.path.join(ROOT, "tools", "perf_probe.py"),
                            "--case", case, "--seconds", SEC, "--warmup", WARM, "--out", out],
                           cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            d = json.load(io.open(out, encoding="utf-8"))[0]
            ms = d["drag"]["metrics"]
            row = {
                "fps": statistics.median([m["fps"] for m in ms]),
                "p50": statistics.median([m["p50_loop"] for m in ms]),
                "p95": statistics.median([m["p95_loop"] for m in ms]),
                "maxloop": max(m["max_loop_ms"] for m in ms),
                "dirty": statistics.median([m["dirty_mpx_avg"] for m in ms]),
                "passa": statistics.median([m["pass_a_mpx_avg"] for m in ms]),
                "bus": statistics.median([m["stage_us"][1] for m in ms]),
                "pus": statistics.median([m["stage_us"][2] for m in ms]),
                "rel": max(m["rel_max_ms"] for m in ms),
                "dda": statistics.median([m["dda_acq"] for m in ms]),
                "cpu": d["drag"]["cpu_pct_of_one_core"],
                "priv": d["idle"]["private_mb"],
                "ws": d["idle"]["working_set_mb"],
            }
            res.setdefault((case, tag), []).append(row)
            print("rep%d %-11s %-6s fps=%5.1f p50=%6.2f p95=%6.2f dirty=%.2f passA=%.2f B=%dus P=%dus cpu=%.2f%% priv=%.1fMB rel=%.1f dda=%.0f"
                  % (rep, case, tag, row["fps"], row["p50"], row["p95"], row["dirty"],
                     row["passa"], row["bus"], row["pus"], row["cpu"], row["priv"], row["rel"], row["dda"]))
            sys.stdout.flush()
            os.remove(out)

print("")
print("=== 中位数汇总（%d 次重复）===" % REP)
for case in CASES:
    a = res[(case, "prefix")]
    b = res[(case, "fixed")]
    print("[%s]" % case)
    for k, unit in (("fps", ""), ("p50", "ms"), ("p95", "ms"), ("dirty", "Mpx"), ("passa", "Mpx"),
                    ("bus", "us"), ("pus", "us"), ("cpu", "%"), ("priv", "MB")):
        va = statistics.median([r[k] for r in a])
        vb = statistics.median([r[k] for r in b])
        d = (vb - va) / va * 100.0 if va else 0.0
        print("   %-6s 修复前 %8.2f%s  修复后 %8.2f%s  变化 %+6.1f%%" % (k, va, unit, vb, unit, d))
json.dump({"%s|%s" % k: v for k, v in res.items()},
          io.open(os.path.join(ROOT, "tools", "ab_summary.json"), "w", encoding="utf-8"),
          ensure_ascii=False, indent=1)
