"""verify_dirty.py -- 脏矩形渲染（Pass A + Pass B）的进程内逐像素自证。

同一进程、同一张桌面 capture 下连渲两帧：
  A) 正常路径：Pass A 只刷新「DDA 脏矩形 ∩ 块足迹」，Pass B 只清/画脏区内的块
  B) 对照路径：Pass A 覆盖整个块足迹，Pass B 整屏清屏 + 全部块重画
两张 overlay 必须逐位一致。测试期间会移动光标并真实拖拽一块玻璃，
把「块移动」「鼠标点光」「窗口区域扩展」全部走进脏区推导。
"""
import ctypes
import glob
import io
import math
import os
import subprocess
import sys
import time

try:
    ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
except Exception:
    ctypes.windll.user32.SetProcessDPIAware()
user32 = ctypes.windll.user32

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# 被测可执行文件 = dev playground 产物；可用 VERRE_EXE 覆盖（例如指向旧基线做 A/B）。
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")
BASE = os.path.join(ROOT, "tools", "dirty")


def load_bmp(path):
    d = io.open(path, "rb").read()
    assert d[:2] == b"BM", path
    off = int.from_bytes(d[10:14], "little")
    w = int.from_bytes(d[18:22], "little", signed=True)
    h = abs(int.from_bytes(d[22:26], "little", signed=True))
    return d[off:], w, h


def compare(tag, left, right):
    a, w, h = load_bmp(left)
    b, w2, h2 = load_bmp(right)
    if (w, h) != (w2, h2):
        print("  %s 尺寸不一致 %dx%d vs %dx%d -> FAIL" % (tag, w, h, w2, h2))
        return False
    diff = 0
    maxd = 0
    sample = []
    for i in range(0, len(a), 4):
        if a[i:i + 4] != b[i:i + 4]:
            diff += 1
            m = max(abs(a[i + k] - b[i + k]) for k in range(3))
            maxd = max(maxd, m)
            if len(sample) < 4:
                sample.append(((i // 4) % w, (i // 4) // w, m))
    ok = diff == 0
    print("  %s %dx%d 差异像素=%d (%.6f%%) 最大通道差=%d %s -> %s"
          % (tag, w, h, diff, 100.0 * diff / (w * h), maxd, sample,
             "PASS" if ok else "FAIL"))
    return ok


def run_case(label, extra_env, drag_target):
    for f in glob.glob(os.path.join(ROOT, "tools", "dirty*")):
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
    env["GLASS_METRICS"] = "1"
    env["GLASS_VERIFY_PASS_A"] = "1"
    env["GLASS_INTERNAL_SHOT"] = BASE
    env["GLASS_INTERNAL_SHOT_AT_MS"] = "6000"
    env.update(extra_env)
    proc = subprocess.Popen([EXE], env=env, cwd=ROOT,
                            stdout=subprocess.DEVNULL,
                            stderr=subprocess.PIPE)
    sw = user32.GetSystemMetrics(0)
    sh = user32.GetSystemMetrics(1)
    t0 = time.perf_counter()
    dragging = False
    # 取证帧一次要落 ~170 MB BMP，必须给足写盘时间，否则会误判 FAIL。
    while time.perf_counter() - t0 < 45.0:
        t = time.perf_counter() - t0
        if drag_target is not None and 2.0 < t < 5.5:
            if not dragging:
                user32.SetCursorPos(int(drag_target[0]), int(drag_target[1]))
                time.sleep(0.2)
                user32.mouse_event(0x0002, 0, 0, 0, 0)
                dragging = True
            cx, cy = drag_target
            x = cx + 380.0 * math.sin((t - 2.0) * 2.1)
            y = cy + 220.0 * math.sin((t - 2.0) * 1.3)
            user32.SetCursorPos(int(x), int(y))
        else:
            if dragging:
                user32.mouse_event(0x0004, 0, 0, 0, 0)
                dragging = False
            x = sw * 0.5 + sw * 0.30 * math.sin(t * 1.9)
            y = sh * 0.5 + sh * 0.28 * math.cos(t * 1.2)
            user32.SetCursorPos(int(x), int(y))
        if glob.glob(BASE + ".m*.full.bmp.overlay.bmp"):
            time.sleep(2.0)
            break
        time.sleep(0.006)
    if dragging:
        user32.mouse_event(0x0004, 0, 0, 0, 0)
    time.sleep(2.0)
    proc.terminate()
    err = proc.stderr.read().decode("utf-8", "replace")
    proc.wait(timeout=10)

    print("[%s]" % label)
    monitors = sorted({p.split(".m")[1][0] for p in glob.glob(BASE + ".m*.bmp")
                       if ".capture." not in p and ".overlay." not in p
                       and ".frost." not in p and ".soft." not in p
                       and ".full." not in p})
    ok = True
    for mi in monitors:
        left = "%s.m%s.bmp.overlay.bmp" % (BASE, mi)
        right = "%s.m%s.full.bmp.overlay.bmp" % (BASE, mi)
        if not (os.path.exists(left) and os.path.exists(right)):
            print("  屏 %s 缺少对照图 (left=%s right=%s)"
                  % (mi, os.path.exists(left), os.path.exists(right)))
            ok = False
            continue
        ok = compare("screen%s" % mi, left, right) and ok
    for line in err.splitlines():
        if "verify" in line or "warn" in line or "info] 显示器" in line:
            print("  " + line)
    return ok


def main():
    results = []
    results.append(run_case("单块 + 拖拽", {}, (1560.0, 1040.0)))
    results.append(run_case("20 块 + 拖拽", {"GLASS_MANY": "1"}, (332.0, 264.0)))
    results.append(run_case("多屏各一块 + 拖拽", {"GLASS_SPREAD": "1"}, (1560.0, 1040.0)))
    print("=" * 46)
    print("总体结论:", "PASS 脏矩形渲染与全量渲染逐位一致"
          if all(results) else "FAIL 存在像素差异")
    return 0 if all(results) else 1


if __name__ == "__main__":
    sys.exit(main())
