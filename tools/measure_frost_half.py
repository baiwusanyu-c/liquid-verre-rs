"""measure_frost_half.py -- GLASS_FROST_HALF 的视觉代价与性能收益量化。

视觉：用确定性合成采集源（GLASS_SYNTH_CAP=1）分别拍两张 overlay，
      逐像素统计差异并输出三联放大对照图（基准 / 半分辨率档 / 差异×8）。
      合成图案含 48px 格纹 + 正弦条带 + 高频噪声，是**比真实桌面更苛刻**的
      最坏情况；真实桌面更平滑，差异只会更小。
性能：round-robin 交错跑 FROST_HALF=0/1，报中位数与极差。
"""
import io
import os
import statistics
import subprocess
import sys
import time

from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "tools"))
# 被测可执行文件 = dev playground 产物；可用 VERRE_EXE 覆盖（例如指向旧基线做 A/B）。
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")
OUT = os.path.join(ROOT, "tools")


def shot(tag, extra, seconds=11.0):
    base = os.path.join(OUT, "fh_%s.bmp" % tag)
    for suffix in ("", ".overlay.bmp", ".capture.bmp", ".frost.bmp", ".soft.bmp"):
        for pre in (base + ".m0.bmp", base + ".m1.bmp"):
            p = pre + suffix
            if os.path.exists(p):
                os.remove(p)
    env = dict(os.environ)
    # # 前台若恰好是「无边框最大化的普通应用」（Electron / Tauri 那一类），库的独占全屏
    # # 检测（方案 R-16）会命中 -> 画布隐藏 + 整屏停产 -> 本脚本拿到 0 帧连拍并假 FAIL。
    # # 取证必须对「用户前台开着哪个应用」保持 hermetic，与 GLASS_ADAPTIVE=0 同理。
    env["GLASS_FS_HIDE"] = "0"
    # 库默认**不**自建玻璃窗口（上层应用负责 create_window）；
    # 取证脚本需要一块确定性的默认玻璃，因此显式打开。
    env["GLASS_DEFAULT_BLOCK"] = "1"
    # 像素等价证明不允许自适应降档中途改视觉（level>=1 会换 frost 分辨率 /
    # 关焦散色散），否则同一份代码两次跑都会对不上。cmp_visual.py 早就关了，
    # 这里补齐其余脚本。
    env["GLASS_ADAPTIVE"] = "0"
    env.update({"GLASS_MANY": "1", "GLASS_SYNTH_CAP": "1",
                "GLASS_INTERNAL_SHOT": base})
    env.update(extra)
    # ⚠ 必须 --no-browser：panel.rs 在不带它时会 `cmd /c start` 打开浏览器，而 playground 被
# terminate 后标签页不会关 => 每次采样累积一个页面，CPU/内存被浏览器吃掉、浏览器还抢前台（前台一被交还就可能落到 `Progman` 上，连带触发独占全屏检测）。
    proc = subprocess.Popen([EXE, "--no-browser"], env=env, cwd=ROOT, stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL)
    time.sleep(seconds)
    proc.terminate()
    try:
        proc.wait(timeout=8)
    except Exception:
        proc.kill()
    time.sleep(2.0)
    return base + ".m0.bmp.overlay.bmp"


def load(path):
    d = io.open(path, "rb").read()
    off = int.from_bytes(d[10:14], "little")
    w = int.from_bytes(d[18:22], "little", signed=True)
    h = abs(int.from_bytes(d[22:26], "little", signed=True))
    return d[off:], w, h


def stats(a_path, b_path):
    A, w, h = load(a_path)
    B, _, _ = load(b_path)
    assert len(A) == len(B)
    glass = 0
    diff = 0
    total_delta = 0
    maxd = 0
    hist = [0] * 8            # 0,1,2,3,4-7,8-15,16-31,>=32
    worst = []
    for i in range(0, len(A), 4):
        if A[i + 3] == 0 and B[i + 3] == 0:
            continue
        glass += 1
        d = max(abs(A[i + k] - B[i + k]) for k in range(3))
        if d:
            diff += 1
            total_delta += d
            maxd = max(maxd, d)
            b = 0 if d == 1 else 1 if d == 2 else 2 if d == 3 else 3 if d < 4 else 4 if d < 8 else 5 if d < 16 else 6 if d < 32 else 7
            hist[b] += 1
            if len(worst) < 400:
                worst.append((d, (i // 4) % w, (i // 4) // w))
    worst.sort(reverse=True)
    return {
        "w": w, "h": h, "glass_px": glass, "diff_px": diff,
        "diff_pct": 100.0 * diff / max(glass, 1),
        "mean_delta": total_delta / max(diff, 1),
        "mean_delta_all": total_delta / max(glass, 1),
        "max_delta": maxd,
        "hist": hist,
        "worst": worst[:6],
    }


def make_comparison(a_path, b_path, st, out_png):
    """三联放大对照：基准 / 半分辨率档 / 差异×8，取两块区域（块内部 + 块边缘）。"""
    A, w, h = load(a_path)
    B, _, _ = load(b_path)
    Z = 3
    crops = []
    # 块 0 在 GLASS_MANY 布局下是 (32,64) 600x400；再取一块中间的
    crops.append(("块内部(模糊主导)", (250, 200, 410, 360)))
    crops.append(("块右边缘(折射+斜面)", (560, 200, 720, 360)))
    crops.append(("差异最大的位置", (max(0, st["worst"][0][1] - 80),
                                    max(0, st["worst"][0][2] - 80),
                                    max(0, st["worst"][0][1] - 80) + 160,
                                    max(0, st["worst"][0][2] - 80) + 160))
               if st["worst"] else ("差异最大的位置", (250, 200, 410, 360)))
    panels = []
    for title, (x0, y0, x1, y1) in crops:
        x0 = max(0, min(x0, w - 2)); x1 = max(x0 + 2, min(x1, w))
        y0 = max(0, min(y0, h - 2)); y1 = max(y0 + 2, min(y1, h))
        cw, ch = x1 - x0, y1 - y0
        ia = Image.new("RGB", (cw * Z, ch * Z))
        ib = Image.new("RGB", (cw * Z, ch * Z))
        idf = Image.new("RGB", (cw * Z, ch * Z))
        pa, pb, pd = ia.load(), ib.load(), idf.load()
        for y in range(ch):
            for x in range(cw):
                i = ((y0 + y) * w + (x0 + x)) * 4
                va = (A[i + 2], A[i + 1], A[i])
                vb = (B[i + 2], B[i + 1], B[i])
                dv = tuple(min(255, abs(A[i + k] - B[i + k]) * 8) for k in (2, 1, 0))
                for dy in range(Z):
                    for dx in range(Z):
                        pa[x * Z + dx, y * Z + dy] = va
                        pb[x * Z + dx, y * Z + dy] = vb
                        pd[x * Z + dx, y * Z + dy] = dv
        row = Image.new("RGB", (cw * Z * 3 + 24, ch * Z + 22), (245, 245, 245))
        row.paste(ia, (0, 22))
        row.paste(ib, (cw * Z + 12, 22))
        row.paste(idf, (cw * Z * 2 + 24, 22))
        panels.append((title, row))
    total_h = sum(p[1].height for p in panels)
    canvas = Image.new("RGB", (max(p[1].width for p in panels), total_h), (255, 255, 255))
    from PIL import ImageDraw
    dr = ImageDraw.Draw(canvas)
    y = 0
    for title, row in panels:
        canvas.paste(row, (0, y))
        dr.text((4, y + 5), title, fill=(0, 0, 0))
        dr.text((row.width // 3 - 60, y + 5), "A: 默认(整屏 frost)", fill=(0, 0, 160))
        dr.text((row.width * 2 // 3 - 60, y + 5), "B: FROST_HALF=1", fill=(0, 0, 160))
        dr.text((row.width - 130, y + 5), "|A-B| x8", fill=(160, 0, 0))
        y += row.height
    canvas.save(out_png)
    return out_png


def main():
    only = os.environ.get("FH_ONLY", "")
    if only != "perf":
        print("[视觉] 拍两张确定性合成采集源下的 overlay ...")
        a = shot("base", {"GLASS_FROST_HALF": "0"})
        b = shot("half", {"GLASS_FROST_HALF": "1"})
        if not (os.path.exists(a) and os.path.exists(b)):
            print("  取证图缺失:", os.path.exists(a), os.path.exists(b))
            return 2
        st = stats(a, b)
        print("  玻璃覆盖像素 = %d" % st["glass_px"])
        print("  有差异的像素 = %d (%.3f%%)" % (st["diff_px"], st["diff_pct"]))
        print("  差异像素的平均通道差 = %.3f / 255" % st["mean_delta"])
        print("  全部玻璃像素的平均通道差 = %.4f / 255" % st["mean_delta_all"])
        print("  最大通道差 = %d / 255" % st["max_delta"])
        labels = ["=1", "=2", "=3", "=4..7", "=8..15", "=16..31", ">=32"]
        print("  差异分布: " + "  ".join(
            "%s:%d(%.2f%%)" % (labels[i], st["hist"][i],
                               100.0 * st["hist"][i] / max(st["glass_px"], 1))
            for i in range(7)))
        print("  差异最大的 6 个像素 (通道差, x, y): %s" % (st["worst"],))
        png = make_comparison(a, b, st, os.path.join(OUT, "frost_half_compare.png"))
        print("  对照图: %s" % png)
        io.open(os.path.join(OUT, "frost_half_stats.txt"), "w",
                encoding="utf-8").write(repr(st))

    if only != "visual":
        print("")
        print("[性能] round-robin 交错 FROST_HALF=0 / 1 ...")
        import importlib.util
        spec = importlib.util.spec_from_file_location(
            "pp", os.path.join(ROOT, "tools", "perf_probe.py"))
        pp = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(pp)

        def avg(ms, k):
            v = [m[k] for m in ms if k in m]
            return sum(v) / len(v) if v else float("nan")

        def med(ms, k):
            v = sorted(x[k] for x in ms if k in x)
            return v[len(v) // 2] if v else float("nan")

        def mx(ms, k):
            v = [m[k] for m in ms if k in m]
            return max(v) if v else float("nan")

        cases = os.environ.get("FH_CASES", "many-drag").split(",")
        reps = int(os.environ.get("FH_REPS", "3"))
        secs = float(os.environ.get("FH_SECONDS", "8"))
        acc = {}
        for rep in range(reps):
            for case in cases:
                for flag in ("0", "1"):
                    old = os.environ.get("GLASS_FROST_HALF")
                    os.environ["GLASS_FROST_HALF"] = flag
                    try:
                        r = pp.run_case(case, secs, 3.0)
                    finally:
                        if old is None:
                            os.environ.pop("GLASS_FROST_HALF", None)
                        else:
                            os.environ["GLASS_FROST_HALF"] = old
                    ph = r.get("drag") if isinstance(r.get("drag"), dict) else r.get("idle")
                    ms = (ph or {}).get("metrics") or []
                    key = (case, flag)
                    acc.setdefault(key, []).append({
                        "fps": avg(ms, "fps"), "cpu": ph.get("cpu_pct_of_one_core"),
                        "p50": med(ms, "p50_loop"), "p95": med(ms, "p95_loop"),
                        "maxloop": mx(ms, "max_loop_ms"),
                        "ren": med(ms, "p50_ren"), "cap": med(ms, "p50_cap"),
                        "passA": avg(ms, "pass_a_mpx_avg"),
                        "priv": (r.get("idle") or {}).get("private_mb"),
                        "ws": (r.get("idle") or {}).get("working_set_mb"),
                    })
                    d = acc[key][-1]
                    print("  rep%d %-11s half=%s fps=%6.1f cpu=%5.2f%% p50loop=%6.2f "
                          "p95loop=%6.2f p50ren=%5.2f passAMpx=%.3f priv=%s"
                          % (rep, case, flag, d["fps"], d["cpu"] or float("nan"),
                             d["p50"], d["p95"], d["ren"], d["passA"], d["priv"]),
                          flush=True)

        def md(vs):
            vs = [v for v in vs if v is not None and v == v]
            return statistics.median(vs) if vs else float("nan")

        print("")
        print("%-11s %6s %8s %8s %7s %7s %7s %8s %8s %8s" % (
            "case", "half", "fps中位", "fps极差", "p50loop", "p95loop",
            "cpu%", "passAMpx", "ws MB", "priv MB"))
        for case in cases:
            row = {}
            for flag in ("0", "1"):
                rs = acc[(case, flag)]
                fps = [r["fps"] for r in rs]
                row[flag] = {
                    "fps": md(fps), "fps_rng": max(fps) - min(fps),
                    "p50": md([r["p50"] for r in rs]),
                    "p95": md([r["p95"] for r in rs]),
                    "cpu": md([r["cpu"] for r in rs]),
                    "passA": md([r["passA"] for r in rs]),
                    "ws": md([r["ws"] for r in rs]),
                    "priv": md([r["priv"] for r in rs]),
                }
                print("%-11s %6s %8.1f %8.1f %7.2f %7.2f %7.2f %8.3f %8.1f %8.1f" % (
                    case, flag, row[flag]["fps"], row[flag]["fps_rng"],
                    row[flag]["p50"], row[flag]["p95"], row[flag]["cpu"],
                    row[flag]["passA"], row[flag]["ws"], row[flag]["priv"]))
            a, b = row["0"], row["1"]
            print("  -> FROST_HALF=1 相对默认: fps %+.1f (%+.0f%%)  p50loop %+.2f ms  "
                  "p95loop %+.2f ms  cpu %+.2f%%  passAMpx %+.3f  priv %+.1f MB"
                  % (b["fps"] - a["fps"],
                     100.0 * (b["fps"] - a["fps"]) / max(a["fps"], .1),
                     b["p50"] - a["p50"], b["p95"] - a["p95"],
                     b["cpu"] - a["cpu"], b["passA"] - a["passA"],
                     b["priv"] - a["priv"]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
