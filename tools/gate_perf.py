# -*- coding: utf-8 -*-
"""gate_perf.py -- 性能阀门：一条命令跑关键性能指标并按阈值判 PASS/FAIL。

## 判据分两档：绝对 vs 同机 A/B 相对

**有参考二进制时（设了 `VERRE_OLD_EXE` 且文件存在）⇒ 硬判据是「相对不劣化」**：
每个场景都测「新构建」与「参考构建」（上一个发布版的 EXE）两臂，**轮转 + 反序 counterbalance
+ 取中位数**，新构建的 fps 不得比参考差过 `REL_MAX_DEGRADE`（3%）。
绝对值照样打印，但从「判据」降级为「参考」：越过绝对门槛时只打一条 `ABS-DRIFT` 提示，
提醒那份绝对数字**失准了、需要重新标定**，而不是让它决定 PASS/FAIL。

**没有参考二进制时 ⇒ 退回绝对硬门槛**（今天的判定表），CI、别的机器、以及忘记建参考时
仍然有卡点。绝对门槛的原文见下面「绝对门槛」一节。

⚠ **为什么不直接把绝对门槛调低放行**：那等于静默削弱闸门 —— 下次真回归时它就不响了。
机器漂移的正解是「和同一台机器上的上一个版本比」，不是「把尺子改短」。

## 绝对门槛（回退档；与 BASELINE.md / performance_report.md 第十三、十四轮一致）

  硬门槛（FAIL 即退出码 1）：
    single-drag  fps >= 90  且 p50 <= 9.0 ms      （120Hz 屏单块必须锁相）
    many-drag    fps >= 60                        （方案 ④ 的下限）
    many-drag + SIM_BACKDROP(900x700@120) + lat=2  fps >= 90
        （确定性负载下测"我们自己的吞吐"；真实桌面的 fps 受桌面重绘率/DWM 背压
          支配，84~105 波动，只作软门槛）
    idle Private < 200 MB                         （方案 ③；**两档模式下都是硬判据**，
          内存不随机器漂移）
  软门槛（只 WARN）：
    many-drag 默认档 fps >= 90（受桌面重绘负载/DWM 背压影响，非渲染成本）
    拖拽 CPU < 5% 单核（同上，安静桌面才稳定达标）

## 已知失准（2026-10-22 实测，务必知道再决定信不信绝对档）

`many-drag (lat=1) >= 60` 这条在本机**没有余量**：0.4.1 基线记录是 60.9（只高 1.5%），
而机器状态一漂移就落到 59.2；同一台机器上关掉被测特性（`GLASS_OCCLUSION=0`）得到
**逐位相同的 59.2** ⇒ 这不是回归，是门槛贴着机器能力线。
⇒ 正确用法：**平时带 `VERRE_OLD_EXE` 走相对档**；绝对档留给"没有参考可比"的场合。
建参考二进制的命令见本文件末尾的 `--help` 输出与 `measurement-and-gates.md §4`。

每次运行都会打印 dda_acq / dirty，便于在 FAIL 时区分「我们慢」还是「桌面/DWM 忙」。
"""
import json
import os
import statistics
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROBE = os.path.join(ROOT, "tools", "perf_probe.py")

# 相对档：新构建相对参考构建允许的 fps 劣化上限（百分比）。
# 3% 是「比噪声大、比任何真实回归小」的折中：本机同臂重复的散布在 0~5% 量级，
# 而真实的渲染回归（少跳一次 Pass A、脏区算漏一块）通常是双位数百分比。
REL_MAX_DEGRADE = float(os.environ.get("GATE_REL_MAX", "3.0"))
# 相对档下每个场景重复几轮。**默认 3 不是折中，而是 spec 的硬口径**
# （`measurement-and-gates.md §2`：性能结论要 >=3 重复 + 中位数 + 轮转）。
# 实测教训：本机 2 轮时臂内散布可达 17%（一度被“新臂莫名快 17.6%”骗到），
# 3% 的门槛会被噪声完全吃掉 => 少一轮不是“省时间”，是“判据失效”。
REPS = max(2, int(os.environ.get("GATE_REPS", "3")))

# 绝对门槛表只在**没有参考二进制**时作为硬判据使用（回退档）。
ABS_GATES = [
    ("single-drag", lambda r: r["fps"] >= 90 and r["p50"] <= 9.0,
     "单块拖拽 fps/p50 未达标: %.1f fps / %.2f ms"),
    ("many-drag", lambda r: r["fps"] >= 60, "20 块默认档 fps < 60: %.1f"),
    ("many-drag-lat2", lambda r: r["fps"] >= 90,
     "20 块吞吐档(lat=2, 合成负载) fps < 90: %.1f"),
    ("transparent-drag", lambda r: r["fps"] >= 90 and r["p50"] <= 9.0,
     "透明模式单块拖拽 fps/p50 未达标: %.1f fps / %.2f ms"),
]


def run(case, extra=None, seconds=10.0, exe=None):
    """跑一个场景。`exe` 非空时覆盖被测二进制（相对 A/B 的参考臂用它）。"""
    env = dict(os.environ)
    if exe:
        env["VERRE_EXE"] = exe
    # # 前台若恰好是「无边框最大化的普通应用」（Electron / Tauri 那一类），库的独占全屏
    # # 检测（方案 R-16）会命中 -> 画布隐藏 + 整屏停产 -> 本脚本拿到 0 帧连拍并假 FAIL。
    # # 取证必须对「用户前台开着哪个应用」保持 hermetic，与 GLASS_ADAPTIVE=0 同理。
    env["GLASS_FS_HIDE"] = "0"
    # 库默认**不**自建玻璃窗口（上层应用负责 create_window）；
    # 取证脚本需要一块确定性的默认玻璃，因此显式打开。
    env["GLASS_DEFAULT_BLOCK"] = "1"
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


# 场景表：key 用来对上 ABS_GATES，label 是打印用的名字，kw 直接喂给 run()
CASES = [
    ("single-drag", "single-drag (lat=默认1)",
     dict(case="single-drag")),
    ("many-drag", "many-drag  (lat=默认1)",
     dict(case="many-drag")),
    ("many-drag-lat2", "many-drag  (lat=2, 合成120Hz负载)",
     dict(case="many-drag", extra={"GLASS_FRAME_LATENCY": "2",
                                   "GLASS_SIM_BACKDROP": "900x700@120"})),
    # 需求 F3.1 / 验收 A9：透明玻璃模式必须满足与液态单块**同一套硬门槛**。
    # 透明模式跳过整条 Pass A，own_us 理论上应明显低于液态；若反而更高说明
    # 跳过链路断了（例如 needs_backdrop 汇总口径错），用 WARN 提示而不是硬判，
    # 因为 own_us 的绝对值受机器状态影响，硬判会制造假 FAIL。
    ("transparent-drag", "transparent-drag (透明模式)",
     dict(case="transparent-drag")),
]


def spread_pct(samples):
    """臂内散布：同一场景逐轮 fps 的 (max-min)/中位数 x 100。

    为什么非有不可：没有它，3% 的相对门槛在噪声 17% 的窗口里就是**抛硬币**，
    不是闸门。散布与劣化量同量级时唯一的诚实结论是「不可信、重测」——
    既不能宣布回归，也不能签字放行。
    """
    vals = [s["fps"] for s in samples if s.get("fps", 0) > 0]
    if len(vals) < 2:
        return 0.0
    med = statistics.median(vals)
    return (max(vals) - min(vals)) / med * 100.0 if med > 0 else 0.0


def reduce_med(samples):
    """对同一场景多次采样按字段取中位数（spec 的统计口径：中位数，不用均值）。"""
    out = {}
    for k in samples[0]:
        out[k] = statistics.median(s[k] for s in samples)
    return out


def main():
    ref = os.environ.get("VERRE_OLD_EXE")
    use_relative = bool(ref) and os.path.exists(ref)
    if ref and not use_relative:
        print("!! VERRE_OLD_EXE 指向的文件不存在：%s" % ref)
        print("!! 退回**绝对门槛档**（相对 A/B 需要两臂都跑得起来）\n")

    new = {}
    refm = {}
    spreads = {}
    if use_relative:
        print("=== 同机 A/B 相对档：新构建 vs 参考 %s ===" % ref)
        print("    %d 轮 x 2 臂 x %d 场景；奇偶轮**反序**跑抵消机器漂移；逐字段取中位数"
              % (REPS, len(CASES)))
        acc = {k: {"new": [], "ref": []} for k, _l, _kw in CASES}
        for rep_i in range(REPS):
            arms = (("new", None), ("ref", ref)) if rep_i % 2 == 0 else (("ref", ref), ("new", None))
            for arm, exe in arms:
                for key, _label, kw in CASES:
                    acc[key][arm].append(run(exe=exe, **kw))
            print("  轮 %d/%d 完成" % (rep_i + 1, REPS))
        for key, _label, _kw in CASES:
            new[key] = reduce_med(acc[key]["new"])
            refm[key] = reduce_med(acc[key]["ref"])
            spreads[key] = (spread_pct(acc[key]["new"]), spread_pct(acc[key]["ref"]))
    else:
        for key, _label, kw in CASES:
            new[key] = run(**kw)

    hard = []
    warn = []
    drift = []
    s0, m1, m2, t0 = (new["single-drag"], new["many-drag"],
                      new["many-drag-lat2"], new["transparent-drag"])

    # ---- 硬判据 1：吞吐 ----
    if use_relative:
        for key, label, _kw in CASES:
            a, b = new[key]["fps"], refm[key]["fps"]
            if b <= 0:
                warn.append("%s 参考臂 fps=%.1f 不可信，跳过该场景的相对判定" % (label, b))
                continue
            degrade = (b - a) / b * 100.0
            sn, sr = spreads.get(key, (0.0, 0.0))
            noise = max(sn, sr)
            if degrade > REL_MAX_DEGRADE:
                if noise >= degrade * 0.8:
                    hard.append("%s **结果不可信**：劣化 %.1f%% 与臂内散布 %.1f%% 同量级。"
                                "换更健康的机器窗口重跑（重启后立刻跑最好），或 GATE_REPS=5 加轮次；"
                                "不要在噪声上签字"
                                % (label, degrade, noise))
                else:
                    hard.append("%s 相对参考劣化 %.1f%%（>%.1f%%，臂内散布仅 %.1f%%）："
                                "新 %.1f fps vs 参考 %.1f fps"
                                % (label, degrade, REL_MAX_DEGRADE, noise, a, b))
    else:
        # 回退档：绝对门槛（原文见模块 docstring）
        for key, judge, msg in ABS_GATES:
            r = new[key]
            if not judge(r):
                hard.append(msg % ((r["fps"], r["p50"]) if "p50" in msg else (r["fps"],)))

    # ---- 硬判据 2：内存（不随机器漂移，两档都硬判）----
    if max(r["priv"] for r in (s0, m1, m2, t0)) >= 200:
        hard.append("Private >= 200 MB: %s" % [round(r["priv"], 1) for r in (s0, m1, m2, t0)])

    # ---- 结构校验与软门槛（两档都跑）----
    if t0["own"] > s0["own"] + 10:
        warn.append("透明模式 own_us=%.0f 高于液态单块 %.0f（跳过 Pass A 后本应更低）："
                    "检查 needs_backdrop 汇总 / FrameInput / compositor 跳过链路"
                    % (t0["own"], s0["own"]))
    if m1["fps"] < 90:
        warn.append("20 块默认档 fps < 90（%.1f）：桌面重绘 %.0f/s、dirty %.2f Mpx 时受触发节拍/DWM 背压限制"
                    % (m1["fps"], m1["dda"], m1["dirty"]))
    if m1["cpu"] >= 5 or s0["cpu"] >= 5:
        warn.append("拖拽 CPU >= 5%% 单核（single %.2f%% / many %.2f%%）：高重绘桌面窗口才会超"
                    % (s0["cpu"], m1["cpu"]))
    # 机器漂移的**症状**：dda 极低说明桌面本身几乎不更新，此时 fps 不可信
    if m1["dda"] < 20 or s0["dda"] < 20:
        warn.append("dda_acq 偏低（single %.0f/s、many %.0f/s）：桌面几乎没更新，"
                    "或采集后端被拒回退 WGC —— 该窗口下的绝对数字不要下结论"
                    % (s0["dda"], m1["dda"]))

    # ---- 绝对值失准提示（相对档下不决定 PASS/FAIL，只提醒该重新标定尺子）----
    if use_relative:
        for key, judge, msg in ABS_GATES:
            r = new[key]
            if not judge(r):
                drift.append(msg % ((r["fps"], r["p50"]) if "p50" in msg else (r["fps"],)))

    # ---- 打印 ----
    hdr = "%%-%ds %%7s %%7s %%7s %%7s %%8s %%7s %%8s" % max(len(l) for _k, l, _x in CASES)
    print("")
    print(hdr % ("场景", "fps", "p50ms", "p95ms", "cpu%", "privMB", "dda/s", "own_us"))
    for key, label, _kw in CASES:
        r = new[key]
        print(hdr % (label, "%.1f" % r["fps"], "%.2f" % r["p50"], "%.2f" % r["p95"],
                     "%.2f" % r["cpu"], "%.1f" % r["priv"], "%.0f" % r["dda"], "%.0f" % r["own"]))

    if use_relative:
        print("")
        print("%-40s %9s %9s %8s" % ("A/B（新 vs 参考）", "新 fps", "参考 fps", "劣化%"))
        for key, label, _kw in CASES:
            a, b = new[key]["fps"], refm[key]["fps"]
            dg = (b - a) / b * 100.0 if b > 0 else float("nan")
            mark = " <== 超阈值" if dg > REL_MAX_DEGRADE else ""
            sn, sr = spreads.get(key, (0.0, 0.0))
            print("%-40s %9.1f %9.1f %7.1f%%  [臂内散布 %.1f%%/%.1f%%]%s"
                  % (label, a, b, dg, sn, sr, mark))
        print("相对门槛：劣化 > %.1f%% 判 FAIL；绝对值仅作参考" % REL_MAX_DEGRADE)

    print("")
    for w in warn:
        print("WARN:", w)
    for d in drift:
        print("ABS-DRIFT（绝对门槛已失准，不决定结论；请重新标定或改走相对档）:", d)
    for h in hard:
        print("FAIL:", h)
    print("")
    print("档位:", "相对 A/B（VERRE_OLD_EXE）" if use_relative else "绝对门槛（回退档）")
    print("结论:", "PASS 性能阀门通过" if not hard else "FAIL 性能阀门未通过")
    return 0 if not hard else 1


if __name__ == "__main__":
    sys.exit(main())
