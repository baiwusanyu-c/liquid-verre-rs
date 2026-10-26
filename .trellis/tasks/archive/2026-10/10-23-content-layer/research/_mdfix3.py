import io
p = r"D:\github-project\liquid-verre-rs\.trellis\tasks\10-23-content-layer\research\mem_delta.py"
c = io.open(p, "r", encoding="utf-8").read()
old = """    samples = []
    t0 = time.time()
    while time.time() - t0 < HOLD:
        time.sleep(0.5)
        if p.poll() is not None:
            break
        if time.time() - t0 >= START:
            samples.append(priv_mb(p.handle))
    p.terminate()"""
new = """    h = k.OpenProcess(0x0410, False, p.pid)
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
    p.terminate()"""
assert old in c
c = c.replace(old, new, 1)
io.open(p, "w", encoding="utf-8", newline="\n").write(c)
print("OpenProcess fix")
