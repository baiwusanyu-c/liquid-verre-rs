import sys, os, subprocess, time, tempfile
EXE = r"D:\github-project\liquid-verre-rs\target\release\liquid-verre-playground.exe"
env = os.environ.copy()
env.update({
    "GLASS_FS_HIDE": "0", "GLASS_MONITORS": "1", "GLASS_ADAPTIVE": "0",
    "GLASS_DEFAULT_BLOCK": "1", "GLASS_SYNTH_CAP": "1", "GLASS_ENV_EMA": "0",
    "GLASS_METRICS": "1", "GLASS_CONTENT_DEMO": "checker",
    "GLASS_INTERNAL_SHOT": os.path.join(tempfile.gettempdir(), "verre_verify_content", "iso_metrics.bmp"),
    "GLASS_INTERNAL_SHOT_AT_MS": "2500", "GLASS_CURSOR_AT": "1560,1900",
})
log = open(os.path.join(tempfile.gettempdir(), "verre_verify_content", "iso_metrics.log"), "wb")
p = subprocess.Popen([EXE, "--no-browser", "--no-demo-pair"], env=env, stdout=log, stderr=log)
time.sleep(9)
p.terminate()
p.wait(timeout=6)
time.sleep(0.3)
with open(os.path.join(tempfile.gettempdir(), "verre_verify_content", "iso_metrics.log"), "r", encoding="utf-8", errors="replace") as f:
    for line in f.read().splitlines():
        if any(k in line for k in ("metrics", "content", "verify", "boot", "warn", "info")):
            print(line[:200])
