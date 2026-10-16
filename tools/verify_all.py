"""verify_all.py -- 全部视觉/正确性取证一次跑完。

1. 跨版本：当前构建 vs 原始提交 82a49e8（确定性合成采集源），overlay + composed 必须逐位一致
2. 脏矩形：受限路径 vs 强制全量路径，同进程同 capture，含真实拖拽（调 verify_dirty.py）
3. 跨屏拖拽 + DPI 换算（调 verify_crossscreen.py）
4. 环境色 EMA 开/关 首帧等价
5. DDA 局部拷贝 vs 整屏参考拷贝

任何一项 FAIL 都会让整体返回非 0。
"""
import io
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "release", "liquid-glass.exe")
BASE_EXE = r"D:\github-project\lg-base\target\release\liquid-glass.exe"
RESULTS = []


def load(path):
    d = io.open(path, "rb").read()
    assert d[:2] == b"BM", path
    off = int.from_bytes(d[10:14], "little")
    w = int.from_bytes(d[18:22], "little", signed=True)
    h = abs(int.from_bytes(d[22:26], "little", signed=True))
    return d[off:], w, h


def cmp_bmp(tag, a, b):
    if not (os.path.exists(a) and os.path.exists(b)):
        print("  %-10s 缺文件 %s / %s -> FAIL" % (tag, os.path.exists(a), os.path.exists(b)))
        RESULTS.append((tag, False))
        return False
    A, w, h = load(a)
    B, w2, h2 = load(b)
    if (w, h) != (w2, h2):
        print("  %-10s 尺寸不一致 %dx%d vs %dx%d -> FAIL" % (tag, w, h, w2, h2))
        RESULTS.append((tag, False))
        return False
    diff = 0
    maxd = 0
    for i in range(0, len(A), 4):
        if A[i:i + 4] != B[i:i + 4]:
            diff += 1
            maxd = max(maxd, max(abs(A[i + k] - B[i + k]) for k in range(3)))
    ok = diff == 0
    print("  %-10s %dx%d 差异=%d (%.6f%%) 最大通道差=%d -> %s"
          % (tag, w, h, diff, 100.0 * diff / (w * h), maxd, "PASS" if ok else "FAIL"))
    RESULTS.append((tag, ok))
    return ok


def run(exe, tag, env_extra, seconds, wait_files):
    for f in wait_files:
        if os.path.exists(f):
            os.remove(f)
    env = dict(os.environ)
    env.update(env_extra)
    p = subprocess.Popen([exe], env=env, cwd=ROOT, stdout=subprocess.DEVNULL,
                         stderr=subprocess.PIPE)
    t0 = time.perf_counter()
    while time.perf_counter() - t0 < seconds:
        if wait_files and all(os.path.exists(f) for f in wait_files):
            break
        time.sleep(0.2)
    time.sleep(0.5)
    p.terminate()
    try:
        err = p.stderr.read().decode("utf-8", "replace")
    except Exception:
        err = ""
    p.wait(timeout=10)
    # 进程退出后 duplication / DComp 资源需要一点时间释放，
    # 否则下一个用例的 DuplicateOutput 可能拿不到而空跑。
    time.sleep(3.0)
    return err


def main():
    print("[1] 跨版本逐像素对比（当前构建 vs 原始提交 82a49e8，确定性合成采集源）")
    if not os.path.exists(BASE_EXE):
        print("  跳过：找不到 %s" % BASE_EXE)
    else:
        common = {"GLASS_MANY": "1", "GLASS_SYNTH_CAP": "1"}
        e1 = dict(common); e1["GLASS_INTERNAL_SHOT"] = os.path.join(ROOT, "tools", "va_base.bmp")
        e2 = dict(common); e2["GLASS_INTERNAL_SHOT"] = os.path.join(ROOT, "tools", "va_new.bmp")
        run(BASE_EXE, "base", e1, 25, [])
        run(EXE, "new", e2, 25, [])
        b = os.path.join(ROOT, "tools", "va_base.bmp")
        n = os.path.join(ROOT, "tools", "va_new.bmp.m0.bmp")
        cmp_bmp("overlay", b + ".overlay.bmp", n + ".overlay.bmp")
        cmp_bmp("composed", b, n)
        cmp_bmp("capture", b + ".capture.bmp", n + ".capture.bmp")

    print("[2] 脏矩形渲染 vs 全量渲染（含真实拖拽，逐屏）")
    r = subprocess.run([sys.executable, os.path.join(ROOT, "tools", "verify_dirty.py")],
                       capture_output=True, text=True, encoding="utf-8", errors="replace")
    for line in (r.stdout or "").splitlines():
        if line.strip():
            print("  " + line.strip())
    RESULTS.append(("dirty-rect", "PASS" in (r.stdout or "") and "FAIL" not in (r.stdout or "")))

    print("[3] 跨屏拖拽 + DPI 换算")
    r = subprocess.run([sys.executable, os.path.join(ROOT, "tools", "verify_crossscreen.py")],
                       capture_output=True, text=True, encoding="utf-8", errors="replace")
    for line in (r.stdout or "").splitlines():
        if line.strip():
            print("  " + line.strip())
    RESULTS.append(("cross-screen", r.returncode == 0))

    print("[4] 环境色 EMA 开/关 等价性（首帧 alpha=1，确定性合成采集源）")
    e = {"GLASS_MANY": "1", "GLASS_SYNTH_CAP": "1"}
    e1 = dict(e); e1["GLASS_ENV_EMA"] = "1"
    e1["GLASS_INTERNAL_SHOT"] = os.path.join(ROOT, "tools", "va_emaon.bmp")
    e2 = dict(e); e2["GLASS_ENV_EMA"] = "0"
    e2["GLASS_INTERNAL_SHOT"] = os.path.join(ROOT, "tools", "va_emaoff.bmp")
    run(EXE, "ema_on", e1, 25, [])
    run(EXE, "ema_off", e2, 25, [])
    cmp_bmp("env-ema",
            os.path.join(ROOT, "tools", "va_emaon.bmp.m0.bmp.overlay.bmp"),
            os.path.join(ROOT, "tools", "va_emaoff.bmp.m0.bmp.overlay.bmp"))

    print("[5] DDA 局部拷贝 vs 整屏参考拷贝（每 120 个内容帧回读比对）")
    e = {"GLASS_MANY": "1", "GLASS_VERIFY_COPY": "1", "GLASS_METRICS": "1"}
    err = run(EXE, "copy", e, 50, [])
    passes = err.count("-> PASS")
    fails = err.count("-> FAIL")
    print("  verify-copy: PASS=%d FAIL=%d" % (passes, fails))
    if passes:
        for line in err.splitlines():
            if "verify-copy" in line:
                print("    " + line.strip())
    RESULTS.append(("partial-copy", fails == 0 and passes > 0))

    print("=" * 60)
    bad = [t for t, ok in RESULTS if not ok]
    for t, ok in RESULTS:
        print("  %-14s %s" % (t, "PASS" if ok else "FAIL"))
    print("总体:", "ALL PASS" if not bad else ("FAIL: " + ", ".join(bad)))
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
