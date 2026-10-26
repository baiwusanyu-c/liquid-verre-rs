import ctypes, io, os, subprocess, time
from ctypes import wintypes
k = ctypes.WinDLL("kernel32")
ps = ctypes.WinDLL("psapi")
class PMC(ctypes.Structure):
    _fields__ = [("cb", wintypes.DWORD), ("pf", wintypes.DWORD),
                 ("a", ctypes.c_size_t), ("ws", ctypes.c_size_t),
                 ("b", ctypes.c_size_t), ("c", ctypes.c_size_t),
                 ("d", ctypes.c_size_t), ("e", ctypes.c_size_t),
                 ("f", ctypes.c_size_t), ("g", ctypes.c_size_t),
                 ("priv", ctypes.c_size_t)]
EXE = r"D:\github-project\liquid-verre-rs\target\release\liquid-verre-playground.exe"
env = dict(os.environ)
env.update({"GLASS_FS_HIDE":"0","GLASS_DEFAULT_BLOCK":"1","GLASS_MONITORS":"1","GLASS_ADAPTIVE":"0"})
log = open(os.devnull, "wb")
p = subprocess.Popen([EXE, "--no-browser", "--no-demo-pair"], env=env, cwd=r"D:\github-project\liquid-verre-rs", stdout=log, stderr=log)
time.sleep(3)
h = k.OpenProcess(0x0410, False, p.pid)
print("handle:", h, "pid:", p.pid, "poll:", p.poll())
m = PMC(); m.cb = ctypes.sizeof(m)
ok = ps.GetProcessMemoryInfo(h, ctypes.byref(m), m.cb)
print("GetProcessMemoryInfo:", ok, "priv MB:", m.priv/1048576.0, "err:", ctypes.GetLastError() if not ok else 0)
p.terminate(); p.wait(timeout=8)
