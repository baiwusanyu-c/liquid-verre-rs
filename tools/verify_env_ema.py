"""verify_env_ema.py -- 环境色 EMA（方案 12.6 / R-20）的等价性取证。

首帧 EMA 没有历史，alpha 强制为 1，着色器走 `return cur` 分支，
因此输出必须与「关闭 EMA」逐位一致 —— 这证明静态桌面下 EMA 不改变任何像素
（lerp(x,x,a) == x 精确成立），只有动态背景才会看到抗闪烁效果。
"""
import io, os, subprocess, sys, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# 被测可执行文件 = dev playground 产物；可用 VERRE_EXE 覆盖（例如指向旧基线做 A/B）。
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")


def shot(tag, ema):
    base = os.path.join(ROOT, "tools", "env_%s.bmp" % tag)
    for suffix in ("", ".overlay.bmp", ".capture.bmp", ".frost.bmp", ".soft.bmp"):
        p = base + (".m0.bmp" if suffix == "" else ".m0.bmp" + suffix[0:] )
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
    env["GLASS_MANY"] = "1"
    env["GLASS_INTERNAL_SHOT"] = base
    env["GLASS_ENV_EMA"] = "1" if ema else "0"
    proc = subprocess.Popen([EXE], env=env, cwd=ROOT, stdout=subprocess.DEVNULL,
                            stderr=subprocess.PIPE)
    time.sleep(9.0)
    proc.terminate()
    try:
        proc.wait(timeout=8)
    except Exception:
        proc.kill()
    return base + ".m0.bmp.overlay.bmp"


def load(path):
    d = io.open(path, "rb").read()
    off = int.from_bytes(d[10:14], "little")
    w = int.from_bytes(d[18:22], "little", signed=True)
    h = abs(int.from_bytes(d[22:26], "little", signed=True))
    return d[off:], w, h


a = shot("ema_on", True)
b = shot("ema_off", False)
A, w, h = load(a)
B, w2, h2 = load(b)
assert (w, h) == (w2, h2), ((w, h), (w2, h2))
diff = 0
maxd = 0
sample = []
for i in range(0, len(A), 4):
    if A[i:i + 4] != B[i:i + 4]:
        diff += 1
        m = max(abs(A[i + k] - B[i + k]) for k in range(3))
        maxd = max(maxd, m)
        if len(sample) < 4:
            sample.append(((i // 4) % w, (i // 4) // w, m))
print("EMA 开 / 关 首帧 overlay: %dx%d 差异像素=%d (%.6f%%) 最大通道差=%d %s"
      % (w, h, diff, 100.0 * diff / (w * h), maxd, sample))
print("结论:", "PASS 静态/首帧下 EMA 与非 EMA 逐位一致" if diff == 0 else "FAIL")
sys.exit(0 if diff == 0 else 1)
