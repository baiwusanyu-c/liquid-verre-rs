"""verify_all.py -- 全部视觉/正确性取证一次跑完。

1. 跨版本：当前构建 vs 原始提交 82a49e8（确定性合成采集源），overlay + composed 必须逐位一致
2. 脏矩形：受限路径 vs 强制全量路径，同进程同 capture，含真实拖拽（调 verify_dirty.py）
3. 跨屏拖拽 + DPI 换算（调 verify_crossscreen.py）
4. 环境色 EMA 开/关 首帧等价
5. DDA 局部拷贝 vs 整屏参考拷贝
6. 0.5.2 #1 border 层序（像素 oracle，调 verify_border_zorder.py）
7. 0.5.2 #4 会话期快照跟随（调 verify_session_snapshot.py）
8. 0.5.2 #5 CS_DBLCLKS 双击相位（调 verify_double_click.py）

任何一项 FAIL 都会让整体返回非 0。
"""
import io
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# 被测可执行文件 = dev playground 产物；可用 VERRE_EXE 覆盖（例如指向旧基线做 A/B）。
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")
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
    env.update(env_extra)
    # ⚠ 必须 --no-browser：panel.rs 在不带它时会 `cmd /c start` 打开浏览器，而 playground 被
# terminate 后标签页不会关 => 每次采样累积一个页面，CPU/内存被浏览器吃掉、浏览器还抢前台（前台一被交还就可能落到 `Progman` 上，连带触发独占全屏检测）。
    p = subprocess.Popen([exe, "--no-browser"], env=env, cwd=ROOT, stdout=subprocess.DEVNULL,
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

    # ---- 0.5.2 三件行为 / 像素取证（宿主实测缺陷的回归位）----
    for tag, script, title in [
        ("border-zorder", "verify_border_zorder.py",
         "[6] 0.5.2 #1 border 受块 z 序遮挡（像素 oracle）"),
        ("session-snap", "verify_session_snapshot.py",
         "[7] 0.5.2 #4 会话期快照跟随 + moving 位翻转可见"),
        ("double-click", "verify_double_click.py",
         "[8] 0.5.2 #5 CS_DBLCLKS：物理双击必须到达 DoubleClick 相位"),
    ]:
        print(title)
        r = subprocess.run([sys.executable, os.path.join(ROOT, "tools", script)],
                           capture_output=True, text=True, encoding="utf-8", errors="replace")
        for line in (r.stdout or "").splitlines():
            if line.strip():
                print("  " + line.strip())
        RESULTS.append((tag, r.returncode == 0))

    print("=" * 60)
    bad = [t for t, ok in RESULTS if not ok]
    for t, ok in RESULTS:
        print("  %-14s %s" % (t, "PASS" if ok else "FAIL"))
    print("总体:", "ALL PASS" if not bad else ("FAIL: " + ", ".join(bad)))
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
