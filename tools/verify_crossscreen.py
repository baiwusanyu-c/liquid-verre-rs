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
# 被测可执行文件 = dev playground 产物；可用 VERRE_EXE 覆盖（例如指向旧基线做 A/B）。
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")
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
# 库默认**不**自建玻璃窗口（上层应用负责 create_window）；
# 取证脚本需要一块确定性的默认玻璃，因此显式打开。
env["GLASS_DEFAULT_BLOCK"] = "1"
# 跨屏拖拽的逐位比对同样不允许自适应降档中途改视觉
env["GLASS_ADAPTIVE"] = "0"
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
# 上一个测试脚本若被强杀，可能把左键留在按下状态；先显式释放两次。
u.mouse_event(0x0004, 0, 0, 0, 0)
time.sleep(0.05)
u.mouse_event(0x0004, 0, 0, 0, 0)
time.sleep(0.2)
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
    if glob.glob(BASE + ".m1.bmp") or glob.glob(BASE + ".m0.bmp"):
        # 主屏取证图出现即说明已经拍完；副屏可能因为没有块而整帧短路。
        if glob.glob(BASE + ".m1.bmp") or time.perf_counter() - t0 > 18.0:
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
    if any(k in line for k in ("显示器", "画布已显示", "全屏", "后端", "verify", "saved")):
        print(" ", line.strip()[:150])
    if "warn" in line or "panic" in line or "render(" in line or "resize(" in line:
        print("  !!", line.strip()[:150]); ok = False
# 主屏：块已经离开 -> 要么没有取证图（该屏无块时整帧短路），要么 overlay 全透明
f0 = "%s.m0.bmp.overlay.bmp" % BASE
if os.path.exists(f0):
    w, h, nz, bb = bbox(f0)
    print("  overlay m0: %dx%d 非零alpha=%d bbox=%s (期望 empty)" % (w, h, nz, bb))
    if nz != 0:
        print("    !! 块没有离开主屏"); ok = False
else:
    print("  overlay m0: 无取证图 —— 该屏已无玻璃块，整帧短路（符合预期）")

# 副屏：块应当出现在这里，且物理尺寸 = 逻辑尺寸 x 0.5 + 2 x 阴影边距
f1 = "%s.m1.bmp.overlay.bmp" % BASE
if not os.path.exists(f1):
    print("  overlay m1: 缺少取证图 -> 块没有跨到副屏"); ok = False
else:
    w, h, nz, bb = bbox(f1)
    size = None if bb is None else (bb[2] - bb[0] + 1, bb[3] - bb[1] + 1)
    print("  overlay m1: %dx%d 非零alpha=%d bbox=%s 尺寸=%s" % (w, h, nz, bb, size))
    if nz == 0 or size is None:
        print("    !! 副屏没有玻璃"); ok = False
    elif not (600 <= size[0] <= 650 and 385 <= size[1] <= 420):
        # 默认单块 = 主屏 1200x760 物理 = 1200x760 逻辑；
        # 落到 scale=0.5 的副屏应变成 600x380 物理 + 2*12 阴影边距。
        print("    !! 副屏物理尺寸不是 1200x760 逻辑 x 0.5 + 阴影边距"); ok = False
    # 副屏采集必须是 DISPLAY2 的内容，不能是主屏
    c0 = "%s.m0.bmp.capture.bmp" % BASE
    c1 = "%s.m1.bmp.capture.bmp" % BASE
    if os.path.exists(c0) and os.path.exists(c1):
        A, wa, ha = load_bmp(c0); B, wb, hb = load_bmp(c1)
        same = tot = 0
        for y in range(0, min(ha, hb), 7):
            for x in range(0, min(wa, wb), 7):
                i0 = (y * wa + x) * 4; i1 = (y * wb + x) * 4
                tot += 1
                if A[i0:i0 + 3] == B[i1:i1 + 3]:
                    same += 1
        pct = 100.0 * same / max(tot, 1)
        print("  副屏采集 vs 主屏采集同区域相同像素 = %.2f%%" % pct)
        if pct > 5.0:
            print("    !! 副屏采集疑似路由到了主屏"); ok = False

print("=" * 44)
print("跨屏拖拽结论:", "PASS" if ok else "FAIL")
sys.exit(0 if ok else 1)
