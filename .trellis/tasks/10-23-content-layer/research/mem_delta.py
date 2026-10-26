# -*- coding: utf-8 -*-
"""mem_delta.py -- AC4: content-texture VRAM delta must be readable and ~sum(w*h*4).

Rotating A/B (off vs checker) like ab_memory.py; median of a steady window.
Texture is B8G8R8A8 (4 B/px); expected for the default block on this machine:
1200 x 760 x 4 = 3,648,000 B = 3.48 MiB (+ driver page granularity).
"""
import ctypes, io, os, statistics, subprocess, sys, time
from ctypes import wintypes

k = ctypes.WinDLL("kernel32")
ps = ctypes.WinDLL("psapi")
ROOT = r"D:\github-project\liquid-verre-rs"  # research-only tool; repo root pinned
EXE = os.environ.get("VERRE_EXE") or os.path.join(ROOT, "target", "release", "liquid-verre-playground.exe")
REP = int(os.environ.get("MD_REP", "3"))
HOLD = float(os.environ.get("MD_HOLD", "18"))
START = float(os.environ.get("MD_START", "6"))


class PMC(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("pf", wintypes.DWORD),
                 ("a", ctypes.c_size_t), ("ws", ctypes.c_size_t),
                 ("b", ctypes.c_size_t), ("c", ctypes.c_size_t),
                 ("d", ctypes.c_size_t), ("e", ctypes.c_size_t),
                 ("f", ctypes.c_size_t), ("g", ctypes.c_size_t),
                 ("priv", ctypes.c_size_t)]


def priv_mb(h):
    m = PMC()
    m.cb = ctypes.sizeof(m)
    if ps.GetProcessMemoryInfo(h, ctypes.byref(m), m.cb):
        return m.priv / 1048576.0
    return float("nan")


def one(tag, checker):
    env = dict(os.environ)
    env["GLASS_FS_HIDE"] = "0"
    env["GLASS_DEFAULT_BLOCK"] = "1"
    env["GLASS_MONITORS"] = "1"
    env["GLASS_ADAPTIVE"] = "0"
    env["GLASS_METRICS"] = "1"
    if checker:
        env["GLASS_CONTENT_DEMO"] = "checker"
    else:
        env.pop("GLASS_CONTENT_DEMO", None)
    HERE = os.path.dirname(os.path.abspath(__file__))
    log = io.open(os.path.join(HERE, "memdelta_%s.log" % tag), "wb")
    p = subprocess.Popen([EXE, "--no-browser", "--no-demo-pair"], env=env, cwd=ROOT,
                         stdout=subprocess.DEVNULL, stderr=log)
    h = k.OpenProcess(0x0410, False, p.pid)
    samples = []
    t0 = time.time()
    while time.time() - t0 < HOLD:
        time.sleep(0.5)
        if p.poll() is not None:
            break
        if time.time() - t0 >= START:
            v = priv_mb(h)
            if v == v and v > 1:
                samples.append(v)
    k.CloseHandle(h)
    p.terminate()
    try:
        p.wait(timeout=8)
    except Exception:
        p.kill()
    log.close()
    return samples


def main():
    off, chk = [], []
    for r in range(REP):
        a = one("%d_off" % r, False)
        b = one("%d_chk" % r, True)
        off += a
        chk += b
        print("rep%d off n=%d median=%.1f MB | checker n=%d median=%.1f MB"
              % (r, len(a), statistics.median(a), len(b), statistics.median(b)))
    mo, mc = statistics.median(off), statistics.median(chk)
    print("=== off median %.1f MB | checker median %.1f MB | delta %+.2f MB ===" % (mo, mc, mc - mo))
    print("expected texture bytes: 1200*760*4 = %.2f MiB (driver page granularity may add a bit)"
          % (1200 * 760 * 4 / 1048576.0))
    return 0


if __name__ == "__main__":
    sys.exit(main())
