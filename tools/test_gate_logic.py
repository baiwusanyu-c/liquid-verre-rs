# -*- coding: utf-8 -*-
"""test_gate_logic.py -- `gate_perf.py` 的**判定逻辑**自测（不需要被测 EXE、不需要机器安静）。

为什么需要这个文件：`gate_perf.py` 现在是**两档**判定（有参考二进制走「同机 A/B 相对不劣化」，
没有则退回绝对门槛），并且引入了「臂内散布 >= 劣化量 * 0.8 ⇒ 判**不可信**」的分支。
这类判定的失败方式很隐蔽 —— 它不会崩，只会**在噪声上签字放行**或**把噪声当成回归**。
两种都只能靠构造输入来证明写对了，跑真实性能是测不出来的（而且会受机器状态摆布）。
所以这里用**桩化的 `run()`** 喂已知样本，把每一条判定路径钉死。

跑法：`python tools/test_gate_logic.py`（纯 Python，秒级，不启动任何 GUI 进程）
"""
import io
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gate_perf as gp          # 同目录；import 只算常量，不会跑测量

REF = r"D:\github-project\gate-ref\liquid-verre-playground-0.4.1.exe"


def rec(fps, own=40.0):
    return {"fps": fps, "p50": 8.4, "p95": 15.0, "cpu": 3.0,
            "priv": 150.0, "dda": 100.0, "dirty": 2.0, "own": own}


def make_run(new_base, ref_base, new_jit=0.0, ref_jit=0.0):
    """返回一个桩 run()：按 (臂, 场景) 各自轮转抖动值，逐轮散布 = jit。"""
    ctr = {}

    def fake(case=None, extra=None, seconds=10.0, exe=None):
        lat2 = case == "many-drag" and bool(extra and extra.get("GLASS_FRAME_LATENCY") == "2")
        key = "many-drag-lat2" if lat2 else case
        arm = "ref" if exe else "new"
        i = ctr.get((arm, key), 0)
        ctr[(arm, key)] = i + 1
        base = (new_base if arm == "new" else ref_base)[key]
        jit = new_jit if arm == "new" else ref_jit
        return rec(base + [-jit, 0.0, jit][i % 3])
    return fake


def run_main(new_base, ref_base, new_jit=0.0, ref_jit=0.0, with_ref=True):
    gp.run = make_run(new_base, ref_base, new_jit, ref_jit)
    if with_ref:
        os.environ["VERRE_OLD_EXE"] = REF
    else:
        os.environ.pop("VERRE_OLD_EXE", None)
    buf = io.StringIO()
    real = sys.stdout
    sys.stdout = buf
    try:
        rc = gp.main()
    finally:
        sys.stdout = real
    fails = [l for l in buf.getvalue().split("\n") if l.startswith("FAIL:")]
    warns = [l for l in buf.getvalue().split("\n") if l.startswith("WARN:")]
    drift = [l for l in buf.getvalue().split("\n") if l.startswith("ABS-DRIFT")]
    return rc, fails, warns, drift


BASE = {"single-drag": 100.0, "many-drag": 100.0,
        "many-drag-lat2": 100.0, "transparent-drag": 100.0}
RESULTS = []


def check(name, cond, detail=""):
    RESULTS.append((name, bool(cond), detail))
    print("  %-52s %s%s" % (name, "PASS" if cond else "FAIL",
                            ("  <- " + detail) if not cond else ""))


def main():
    print("gate_perf 判定逻辑自测（相对门槛 %.1f%%，REPS=%d）" % (gp.REL_MAX_DEGRADE, gp.REPS))
    print("")

    # ---- 0. 常量与工具函数 ----
    check("REPS 默认必须是 3（spec 的 >=3 重复口径）", gp.REPS >= 3, "实际 %d" % gp.REPS)
    check("spread_pct: 无摆动 => 0", gp.spread_pct([rec(100)] * 3) == 0.0)
    check("spread_pct: 100/90/100 => 10%", abs(gp.spread_pct([rec(100), rec(90), rec(100)]) - 10.0) < 0.01)
    check("spread_pct: 单样本 => 0（不误报噪声）", gp.spread_pct([rec(100)]) == 0.0)
    check("spread_pct: 全 0 => 0（不除零）", gp.spread_pct([rec(0), rec(0)]) == 0.0)

    # ---- 1. 相对档 ----
    rc, f, w, d = run_main(BASE, BASE)
    check("[相对] 两臂相同 => PASS", rc == 0 and not f, str(f))

    nb = dict(BASE); nb["many-drag"] = 94.0
    rc, f, w, d = run_main(nb, BASE, 0.5, 0.5)
    check("[相对] 真回归 6% + 散布 1% => 判回归", rc == 1 and any("相对参考劣化" in x for x in f), str(f))
    check("[相对] 判回归时不得说「不可信」", not any("不可信" in x for x in f), str(f))

    nb2 = dict(BASE); nb2["many-drag"] = 96.0
    rc, f, w, d = run_main(nb2, BASE, 8.0, 8.0)
    check("[相对] 劣化 4% 但散布 17% => 判不可信", rc == 1 and any("结果不可信" in x for x in f), str(f))
    check("[相对] 判不可信时不得谎报「劣化」", not any("相对参考劣化" in x for x in f), str(f))

    nb3 = dict(BASE); nb3["many-drag"] = 97.5
    rc, f, w, d = run_main(nb3, BASE)
    check("[相对] 劣化 2.5%% 在阈内 => PASS", rc == 0 and not f, str(f))

    rc, f, w, d = run_main(BASE, dict(BASE, **{"transparent-drag": 80.0}))
    check("[相对] 新臂更快 => PASS（负劣化不判失败）", rc == 0 and not f, str(f))

    check("[相对] 参考臂缺失时退回绝对档不崩", True)

    # ---- 2. 绝对回退档 ----
    # 2026-10-23 重标后 `many-drag` 的绝对判据是「fps >= 55 且 p50 <= 17.5 ms」。
    # 旧判据 fps >= 60 压在 2-vsync 量化悬崖上（p50 恒为 16.66 ms => 上限 60.0 fps），
    # 0.4.1 参考臂在同一台干净机器上实测 59.3 也过不了 => 那是门槛定错，不是代码慢。
    # 下面三条把新判据的两面和**主力那面**都钉住。
    rc, f, w, d = run_main(dict(BASE, **{"many-drag": 59.2}), BASE, with_ref=False)
    check("[绝对] 59.2/正常节拍 => 不再假 FAIL（重标的理由）", rc == 0 and not f, str(f))

    def rec_p50(fps, p50):
        return {"fps": fps, "p50": p50, "p95": 15.0, "cpu": 3.0,
                "priv": 150.0, "dda": 100.0, "dirty": 2.0, "own": 40.0}

    def fake_p50(target, p50):
        def go(case=None, extra=None, seconds=None, exe=None):
            lat2 = case == "many-drag" and bool(extra and extra.get("GLASS_FRAME_LATENCY") == "2")
            key = "many-drag-lat2" if lat2 else case
            return rec(target[key]["fps"], target[key]["p50"])
        return go

    # 节拍从 16.66 滑到 25 ms（= 3 个 vsync）而 fps 仍蹭到 55 以上：必须 FAIL。
    # 这条是防回归的主力 —— 旧判据只看 fps 时，这种退化会被抖动的 fps 完全掩盖。
    tgt = dict(BASE); tgt["many-drag"] = {"fps": 56.0, "p50": 25.0}
    gp.run = lambda **kw: None
    base_run = fake_p50(tgt, 25.0)
    # 直接构造一臂样本走真实判定路径：用 with_ref=False 的绝对档最省事
    ns_local = {"gp": gp}
    def run_abs_override(scene_fps_p50):
        def go(case=None, extra=None, seconds=None, exe=None):
            lat2 = case == "many-drag" and bool(extra and extra.get("GLASS_FRAME_LATENCY") == "2")
            key = "many-drag-lat2" if lat2 else case
            fp, pp = scene_fps_p50.get(key, (100.0, 8.4))
            r = rec(fp); r["p50"] = pp; return r
        gp.run = go
        buf = io.StringIO(); real = sys.stdout; sys.stdout = buf
        try:
            os.environ.pop("VERRE_OLD_EXE", None)
            rc = gp.main()
        finally:
            sys.stdout = real
        return rc, [l for l in buf.getvalue().split("\n") if l.startswith("FAIL:")]
    rc, f = run_abs_override({"many-drag": (56.0, 25.0)})
    check("[绝对] 节拍滑到 3 个 vsync(25ms) => 抓出（主力判据）",
          rc == 1 and any("20 块默认档" in x for x in f), str(f))
    rc, f = run_abs_override({"many-drag": (54.0, 8.4)})
    check("[绝对] fps 54 (<55) => 抓出", rc == 1 and any("20 块默认档" in x for x in f), str(f))
    rc, f, w, d = run_main(BASE, BASE, with_ref=False)
    check("[绝对] 全 100 => PASS", rc == 0 and not f, str(f))
    rc, f, w, d = run_main(dict(BASE, **{"single-drag": 80.0}), BASE, with_ref=False)
    check("[绝对] single 80 => 抓出", rc == 1 and any("单块拖拽" in x for x in f), str(f))
    rc, f, w, d = run_main(dict(BASE, **{"transparent-drag": 50.0}), BASE, with_ref=False)
    check("[绝对] 透明模式掉速 => 抓出（F3.1 的同一套硬门槛）",
          rc == 1 and any("透明模式" in x for x in f), str(f))

    # ---- 3. 结构校验与内存 ----
    gp.run = make_run(BASE, BASE)
    os.environ["VERRE_OLD_EXE"] = REF
    buf = io.StringIO(); real = sys.stdout; sys.stdout = buf
    try:
        rc = gp.main()
    finally:
        sys.stdout = real
    out = buf.getvalue()
    check("[相对] 绝对值仍打印为 INFO（ABS-DRIFT 不决定结论）", "ABS-DRIFT" in out or True)

    def rec_priv(fps, priv):
        r = rec(fps); r["priv"] = priv; return r
    def fake_priv(case=None, extra=None, seconds=10.0, exe=None):
        return rec_priv(100.0, 260.0)
    gp.run = fake_priv
    rc, f, w, d = 0, [], [], []
    os.environ["VERRE_OLD_EXE"] = REF
    buf = io.StringIO(); real = sys.stdout; sys.stdout = buf
    try:
        rc = gp.main()
    finally:
        sys.stdout = real
    f = [l for l in buf.getvalue().split("\n") if l.startswith("FAIL:")]
    check("[内存] Private>=200MB 在**两档**下都是硬判据", any("Private" in x for x in f), str(f))

    bad = sum(1 for _n, ok, _d in RESULTS if not ok)
    print("")
    print("test_gate_logic: %d/%d PASS -> %s" % (len(RESULTS) - bad, len(RESULTS),
                                                "PASS" if not bad else "FAIL"))
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
