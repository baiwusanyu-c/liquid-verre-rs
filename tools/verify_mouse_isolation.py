"""verify_mouse_isolation.py -- 「拖/移一块，其余块不得有任何视觉变化」回归测试。

机制：GLASS_SYNTH_CAP 让采集内容确定性；GLASS_CURSOR_AT 固定首帧光标位置；
首帧取证图在显示窗口之前拍出，期间没有任何真实鼠标移动。
于是两次运行之间唯一的变量就是"光标在哪个位置"。

三个场景：
  [1] 20 块密集布局，光标在 (100,1900) 与 (3000,1900)（都远离全部块）
      -> 20 块必须**逐位一致**：证明光标在远处移动不再改变任何块的渲染结果。
      （修复前 normalizedMouse 无距离衰减，这两张图会有大面积差异。）
  [2] 20 块密集布局，光标移到块 0 中心
      -> 块 0 必须变化（proximity 反馈仍在工作）；
         其余块中，只有「gate 支撑半径内 ∪ 沿重叠块阴影传递闭包」的允许变化，
         闭包之外必须逐位一致。
      注意：这个布局块与块大量重叠，闭包常常吞掉全部 20 块，判别力有限，
      因此真正的硬证明是 [3]。
  [3] 6 块互不重叠、两两中心距 >=800px（> 3*mouse_radius=660）的分离布局
      -> 光标落在块 0 中心时，其余 3 块必须**逐位一致**（0 差异像素）。
      这就是用户报的现象的直接判据：拖一块，其它块不许有任何视觉变化。
"""
import io
import math
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# 被测可执行文件 = dev playground 产物；可用 VERRE_EXE 覆盖（例如指向旧基线做 A/B）。
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")
SW, SH = 3120, 2080
MARGIN = 30
# 与 shader 的 gate 支撑半径（3*mouse_radius=660）+ 余量对齐：
# 该半径之外的块必须逐位不受光标影响；半径之内属于方案需求 4 的"邻近余晖"。
ISOLATION_RADIUS = 700.0


def blocks():
    """GLASS_MANY=1 的 20 块密集布局（必须与 src/main.rs 完全一致）。"""
    out = []
    for i in range(20):
        w = 600.0 + (i % 4) * 30.0
        h = 400.0 + (i % 3) * 24.0
        gap = 22.0
        step_x = max(w * 0.62 + gap, 96.0)
        step_y = max(h * 0.60 + gap, 84.0)
        cols = min(max(int((SW - 40.0) // step_x), 1), 8)
        col = i % cols
        row = (i // cols) % 12
        x = min(32.0 + col * step_x, max(SW - w - 24.0, 8.0))
        y = min(64.0 + row * step_y, max(SH - h - 32.0, 8.0))
        out.append((int(x), int(y), int(w), int(h)))
    return out


def far_blocks():
    """GLASS_FAR_GRID=1 的 4 块分离布局（必须与 src/main.rs 完全一致）。

    块 500x340，左上角 (600,300)/(1800,300)/(600,1300)/(1800,1300)。
    探针光标取块 0 内的偏心点 (700,400)（不能取正中心：那里 normalizedMouse=0，
    渐变项本来就中性，探针会退化成"看不出反馈是否还在工作"）。
    此时其余 3 块：中心距 1080~1640px（> gate 支撑半径 660 -> 渐变项精确为 0），
    最近边缘距 900~1423px（fall<=2.4e-4 -> glint*hoverGain 相乘后 <=1e-8，8bit 必为 0）。
    远处参照光标取 (3050,2000)：距所有块边缘 >=832px，fall<=7.8e-4，同样为 0。
    """
    out = []
    for (bx, by) in ((600, 300), (1800, 300), (600, 1300), (1800, 1300)):
        out.append((bx, by, 500, 340))
    return out


def load(path):
    d = io.open(path, "rb").read()
    off = int.from_bytes(d[10:14], "little")
    w = int.from_bytes(d[18:22], "little", signed=True)
    h = abs(int.from_bytes(d[22:26], "little", signed=True))
    return d[off:], w, h


def shot(tag, cursor, far_grid=False):
    base = os.path.join(ROOT, "tools", "mi_%s.bmp" % tag)
    env = dict(os.environ)
    # 库默认**不**自建玻璃窗口（上层应用负责 create_window）；
    # 取证脚本需要一块确定性的默认玻璃，因此显式打开。
    env["GLASS_DEFAULT_BLOCK"] = "1"
    # 像素等价证明不允许自适应降档中途改视觉（level>=1 会换 frost 分辨率 /
    # 关焦散色散），否则同一份代码两次跑都会对不上。cmp_visual.py 早就关了，
    # 这里补齐其余脚本。
    env["GLASS_ADAPTIVE"] = "0"
    env.update({"GLASS_MANY": "1", "GLASS_SYNTH_CAP": "1",
                "GLASS_MONITORS": "1",
                "GLASS_INTERNAL_SHOT": base,
                "GLASS_CURSOR_AT": "%d,%d" % cursor})
    if far_grid:
        env["GLASS_FAR_GRID"] = "1"
    p = subprocess.Popen([EXE], env=env, cwd=ROOT, stdout=subprocess.DEVNULL,
                         stderr=subprocess.DEVNULL)
    time.sleep(9.0)
    p.terminate()
    try:
        p.wait(timeout=8)
    except Exception:
        p.kill()
    time.sleep(1.0)
    f = base + ".m0.bmp.overlay.bmp"
    assert os.path.exists(f), f
    return f


def center(r):
    return (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)


def region_diff(pa, pb, rects):
    """按块统计差异像素。行级 bytes 比较（C 速度），只有真有差异的行才逐像素展开。"""
    A, w, h = load(pa)
    B, _, _ = load(pb)
    stride = w * 4
    bad = []
    for (x, y, bw, bh) in rects:
        x0, y0 = max(0, x - MARGIN), max(0, y - MARGIN)
        x1, y1 = min(w, x + bw + MARGIN), min(h, y + bh + MARGIN)
        diff = 0
        maxd = 0
        for yy in range(y0, y1):
            i0 = yy * stride + x0 * 4
            i1 = yy * stride + x1 * 4
            ra = A[i0:i1]
            if ra == B[i0:i1]:
                continue
            rb = B[i0:i1]
            for k in range(0, len(ra), 4):
                if ra[k:k + 4] != rb[k:k + 4]:
                    diff += 1
                    d = max(abs(ra[k + j] - rb[k + j]) for j in range(3))
                    if d > maxd:
                        maxd = d
        if diff:
            bad.append((x, y, diff, maxd))
    return bad


def closure(rects, seed):
    """seed 集合沿"矩形重叠"关系的传递闭包（余晖会透过重叠块的半透明阴影传播）。"""
    aff = list(seed)
    changed = True
    while changed:
        changed = False
        for i, r in enumerate(rects):
            if i in aff:
                continue
            for j in aff:
                o = rects[j]
                if (r[0] < o[0] + o[2] and o[0] < r[0] + r[2]
                        and r[1] < o[1] + o[3] and o[1] < r[1] + r[3]):
                    aff.append(i)
                    changed = True
                    break
    return aff


def cleanup(*files):
    for f in files:
        for sfx in ("", ".capture.bmp", ".frost.bmp", ".soft.bmp"):
            p2 = f.replace(".overlay.bmp", sfx) if sfx else f.replace(".overlay.bmp", "")
            try:
                if os.path.exists(p2):
                    os.remove(p2)
            except OSError:
                pass


def main():
    rects = blocks()
    print("拍 5 张首帧取证图（20 块密集布局 3 张 + 4 块分离布局 2 张）...")
    fa = shot("a", (100, 1900))
    fb = shot("b", (3000, 1900))
    fc = shot("c", (332, 264))
    ga = shot("g1", (3050, 2000), far_grid=True)
    gc = shot("g2", (700, 400), far_grid=True)

    print("")
    print("[1] 20 块密集布局，光标在两个不同远处：全部 20 块必须逐位一致")
    bad = region_diff(fa, fb, rects)
    if bad:
        for b in bad[:8]:
            print("    !! 块@(%d,%d) 差异像素=%d 最大通道差=%d" % b)
        print("    结论: FAIL（光标移动仍会影响非 hover 块）")
        r1 = False
    else:
        print("    20/20 块 0 差异")
        print("    结论: PASS")
        r1 = True

    print("")
    print("[2] 20 块密集布局，光标移到块 0 上：%dpx 内的邻近块允许有余晖（方案需求 4），"
          % ISOLATION_RADIUS)
    print("    其余块（含沿重叠阴影的传播闭包之外）必须与 [A] 逐位一致")
    c0 = center(rects[0])
    seed = [i for i, r in enumerate(rects)
            if math.hypot(center(r)[0] - c0[0], center(r)[1] - c0[1]) <= ISOLATION_RADIUS]
    affected = closure(rects, seed)
    far = [r for i, r in enumerate(rects) if i not in affected]
    near = [r for i, r in enumerate(rects) if i in affected]
    bad2 = region_diff(fa, fc, far)
    dnear = region_diff(fa, fc, near)
    d0 = region_diff(fa, fc, rects[:1])
    print("    块 0 差异像素=%d（>0 才说明 proximity 反馈仍在工作）"
          % (sum(b[2] for b in d0)))
    print("    余晖及其重叠传播涉及 %d 块，差异像素=%d（允许）"
          % (len(near), sum(b[2] for b in dnear)))
    print("    传播闭包之外 %d 块，要求逐位一致" % len(far))
    if bad2:
        for b in bad2[:8]:
            print("    !! 块@(%d,%d) 差异像素=%d 最大通道差=%d" % b)
        print("    结论: FAIL")
        r2 = False
    else:
        print("    闭包外 %d 块 0 差异" % len(far))
        print("    结论: PASS" if d0 else "    结论: PASS（但块0也无差异，proximity 可能失效，需检查）")
        r2 = bool(d0)

    print("")
    print("[3] 4 块分离布局（互不重叠；光标在块 0 内偏心点 (700,400)）：")
    print("    其余 3 块中心距 1080~1640px > gate 支撑半径 660px，最近边缘距 >=900px")
    print("    -> 必须逐位一致（0 差异像素）—— 这是用户报的现象的直接判据")
    frects = far_blocks()
    dother = region_diff(ga, gc, frects[1:])
    dself = region_diff(ga, gc, frects[:1])
    print("    被拖的块 0 差异像素=%d 最大通道差=%s（必须 >0，证明 proximity 反馈没被一刀切掉）"
          % (sum(b[2] for b in dself),
             max([b[3] for b in dself], default=0)))
    if dother:
        for b in dother[:8]:
            print("    !! 块@(%d,%d) 差异像素=%d 最大通道差=%d" % b)
        print("    结论: FAIL（拖一块，别的块跟着变了）")
        r3 = False
    elif not dself:
        print("    其余 5 块 0 差异，但块 0 也 0 差异 -> proximity 反馈失效")
        print("    结论: FAIL")
        r3 = False
    else:
        print("    其余 3/3 块 0 差异（完全不受影响）")
        print("    结论: PASS")
        r3 = True

    cleanup(fa, fb, fc, ga, gc)
    print("")
    ok = r1 and r2 and r3
    print("总体:", "PASS 非 hover 块完全不受光标移动/拖拽影响" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
