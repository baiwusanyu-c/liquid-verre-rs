"""verify_crossscreen.py -- 跨屏拖拽验收（方案 12.2.5 / 12.10）。

把玻璃从主屏（3120x2080 @200%）拖到副屏（1920x1080 @100%），
在拖过去之后才拍内部取证图，验证：
  1) 块确实换到了副屏画布（主屏 overlay 应为空）；
  2) 尺寸按两屏 DPI 比例换算（600x400 逻辑 -> 300x200 物理）；
  3) 全程无 panic / 无渲染错误。
"""
import ctypes, glob, io, os, subprocess, sys, time
try:
    ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
except Exception:
    ctypes.windll.user32.SetProcessDPIAware()
u = ctypes.windll.user32
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "release", "liquid-glass.exe")
BASE = os.path.join(ROOT, "tools", "cross")

def load(p):
    d = io.open(p, "rb").read()
    off = int.from_bytes(d[10:14], "little")
    w = int.from_bytes(d[18:22], "little", signed=True)
    h = abs(int.from_bytes(d[22:26], "little", signed=True))
    return d[off:], w, h

def bbox(p):
    A, w, h = load(p)
    xs = []; ys = []; nz = 0
    for y in range(h):
        row = y * w * 4
        for x in range(w):
            if A[row + x * 4 + 3]:
                nz += 1; xs.append(x); ys.append(y)
    if not nz:
        return w, h, 0, None
    return w, h, nz, (min(xs), min(ys), max(xs), max(ys))

for f in glob.glob(BASE + "*"):
    os.remove(f)
env = dict(os.environ)
env["GLASS_INTERNAL_SHOT"] = BASE
env["GLASS_INTERNAL_SHOT_AT_MS"] = "14000"
LOG = os.path.join(ROOT, "tools", "cross.log")
if os.path.exists(LOG):
    os.remove(LOG)
logfh = io.open(LOG, "wb")
p = subprocess.Popen([EXE], env=env, cwd=ROOT, stdout=subprocess.DEVNULL,
                     stderr=logfh)
# 等两块屏的画布都真正显示出来再开始拖，否则按下时窗口还不存在。
t0 = time.perf_counter()
while time.perf_counter() - t0 < 20.0:
    logfh.flush()
    txt = io.open(LOG, "r", encoding="utf-8", errors="replace").read()
    if txt.count("画布已显示") >= 2 or "该屏不显示玻璃" in txt:
        break
    time.sleep(0.1)
time.sleep(0.4)
t0 = time.perf_counter()
# 主屏单块默认居中：3120x2080 的 62%/68% 夹紧到 1200x760 -> 中心 (1560,1040)
start = (1560, 1040)
end = (4080, 540)          # 副屏 (3120..5040, 0..1080) 内部
pressed = False
while time.perf_counter() - t0 < 24.0:
    t = time.perf_counter() - t0
    if 1.5 < t < 6.5:
        if not pressed:
            u.SetCursorPos(*start); time.sleep(0.25)
            u.mouse_event(0x0002, 0, 0, 0, 0); pressed = True
        k = (t - 1.5) / 5.0
        u.SetCursorPos(int(start[0] + (end[0] - start[0]) * k),
                       int(start[1] + (end[1] - start[1]) * k))
    elif pressed and t >= 6.5:
        u.mouse_event(0x0004, 0, 0, 0, 0); pressed = False
    if glob.glob(BASE + ".m1.bmp"):
        break
    time.sleep(0.008)
if pressed:
    u.mouse_event(0x0004, 0, 0, 0, 0)
time.sleep(1.0)
p.terminate()
p.wait(timeout=10)
logfh.close()
err = io.open(LOG, "r", encoding="utf-8", errors="replace").read()

ok = True
for line in err.splitlines():
    if "显示器" in line or "画布已显示" in line:
        print(" ", line)
    if "warn" in line or "panic" in line or "render(" in line or "resize(" in line:
        print("  !!", line); ok = False
for mi, expect in (("m0", "empty"), ("m1", "block")):
    f = "%s.%s.bmp.overlay.bmp" % (BASE, mi)
    if not os.path.exists(f):
        print("  %s 缺少 overlay 取证图" % mi); ok = False; continue
    w, h, nz, bb = bbox(f)
    size = None if bb is None else (bb[2] - bb[0] + 1, bb[3] - bb[1] + 1)
    print("  overlay %s: %dx%d 非零alpha=%d bbox=%s 尺寸=%s (期望 %s)"
          % (mi, w, h, nz, bb, size, expect))
    if expect == "empty" and nz != 0:
        ok = False
    if expect == "block":
        if nz == 0 or size is None:
            ok = False
        elif not (600 <= size[0] <= 650 and 385 <= size[1] <= 420):
            # 默认单块 = 主屏 1200x760 物理 = 1200x760 逻辑；
            # 落到 scale=0.5 的副屏应变成 600x380 物理 + 2*12 阴影边距。
            print("    !! 副屏物理尺寸不是 1200x760 逻辑 * 0.5 + 阴影边距"); ok = False
print("=" * 44)
print("跨屏拖拽结论:", "PASS" if ok else "FAIL")
sys.exit(0 if ok else 1)
