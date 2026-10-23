# -*- coding: utf-8 -*-
"""verify_validity.py -- 「漏更新」自检：金字塔有效性虚报 + 该重画的块被跳过。

用户报的四个现象（背景失真/过曝、鼠标在玻璃内移动背景颤抖、多块时拖一块
其余块背景闪动）只可能来自两类"漏更新"，本脚本把它们逼出来：

  * GLASS_SIM_BACKDROP=900x700@60 -> 每帧 backdrop_changed=true（真实桌面本来
    也是 27~116 次/秒在变，这里只是把它确定化）；
  * 光标**跳变**移动 -> 块足迹一跳就超出 192px 迟滞区 -> !covered；
    同时跳变让 drag_velocity 变大 -> pad 从 128 涨到 256 -> 足迹进一步外扩。
    两个条件同时成立时，旧实现会「只刷桌面脏区∩足迹，却声称整个足迹有效」。

判据：程序自己打印的 `[verify] 漏更新自检：...` 计数必须为 0。

为什么不用「增量渲染 == 全量渲染」的逐像素判据（曾经试过）：在 SIM_BACKDROP 场景下
它**两种配置都不成立**——
  * 不用合成源：SIM 只是伪造脏矩形，真实桌面在假脏区之外仍在变，金字塔在那些区域
    合理地保持陈旧，与全量重算必然有差（那是场景性质，不是缺陷）；
  * 用合成源：合成源的 dirty_union 恒为 [0,0,0,0]，金字塔因此永远不跟踪合成图案，
    模糊层停留在首帧的真实桌面上，差异更大（实测 13%~45% 像素）。
所以这个场景的正确判据是**计数器**（虚报/漏画必须为 0），像素一致性交给
verify_dirty（真实桌面、无假脏区）负责。
"""
import ctypes
import io
import math
import os
import subprocess
import sys
import time

try:
    ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
except Exception:
    pass
u = ctypes.windll.user32
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# 被测可执行文件 = dev playground 产物；可用 VERRE_EXE 覆盖（例如指向旧基线做 A/B）。
EXE = os.environ.get("VERRE_EXE") or os.path.join(
    ROOT, "target", "release", "liquid-verre-playground.exe")
LOG = os.path.join(ROOT, "tools", "vv.log")


def main():
    many = os.environ.get("VV_MANY", "1")
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
    env.update({"GLASS_MANY": many, "GLASS_METRICS": "1",
                "GLASS_VERIFY_VALIDITY": "1",
                "GLASS_SIM_BACKDROP": os.environ.get("VV_BACKDROP", "900x700@60"),
                "GLASS_MONITORS": "1"})
    if os.path.exists(LOG):
        os.remove(LOG)
    fh = io.open(LOG, "wb")
    # ⚠ 必须 --no-browser：panel.rs 在不带它时会 `cmd /c start` 打开浏览器，而 playground 被
# terminate 后标签页不会关 => 每次采样累积一个页面，CPU/内存被浏览器吃掉、浏览器还抢前台（前台一被交还就可能落到 `Progman` 上，连带触发独占全屏检测）。
    p = subprocess.Popen([EXE, "--no-browser"], env=env, cwd=ROOT, stdout=subprocess.DEVNULL, stderr=fh)
    t0 = time.perf_counter()
    pressed = False
    jumps = 0
    try:
        while time.perf_counter() - t0 < 14.0:
            t = time.perf_counter() - t0
            if t > 7.0 and not pressed:
                u.SetCursorPos(332, 264)
                time.sleep(0.2)
                u.mouse_event(0x0002, 0, 0, 0, 0)
                pressed = True
            # 每 90 ms 一次 400~700px 的跳变，其余时间小幅抖动
            phase = int(t / 0.09)
            if phase % 2 == 0:
                bx, by = 1560, 1040
            else:
                bx, by = 1560 + 620, 1040 + 430
            x = int(bx + 26 * math.sin(t * 31.0))
            y = int(by + 22 * math.cos(t * 27.0))
            if pressed:
                x = int(332 + (bx - 1560) + 26 * math.sin(t * 31.0))
                y = int(264 + (by - 1040) + 22 * math.cos(t * 27.0))
            u.SetCursorPos(x, y)
            jumps += 1
            time.sleep(0.006)
    finally:
        if pressed:
            u.mouse_event(0x0004, 0, 0, 0, 0)
        p.terminate()
        try:
            p.wait(timeout=10)
        except Exception:
            p.kill()
        fh.close()
    txt = io.open(LOG, "r", encoding="utf-8", errors="replace").read()
    lines = [l.strip() for l in txt.splitlines() if "[verify]" in l]
    print("光标设置次数=%d（含跳变）" % jumps)
    if lines:
        print("最后一条自检:", lines[-1])
    over = skip = 0
    for l in lines:
        try:
            a = l.split("有效")[1].split("帧")[0].strip()
            b = l.split("块")[1].split("次")[0].strip()
            over, skip = int(a), int(b)
        except Exception:
            pass
    print("金字塔虚报有效 = %d 帧" % over)
    print("该重画却漏掉的块 = %d 次" % skip)
    dd = [l.strip() for l in txt.splitlines() if "dda_acq" in l or "pass_a=" in l]
    if dd:
        print("最后一窗 metrics:", dd[-1][:200])
    ok = (over == 0 and skip == 0)
    print("结论:", "PASS 没有漏更新" if ok else "FAIL 存在漏更新（会被用户看成背景闪动/颤抖）")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
