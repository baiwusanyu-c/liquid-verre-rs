import io, os
p = r"D:\github-project\liquid-verre-rs\.trellis\tasks\10-23-content-layer\research\mem_delta.py"
raw = io.open(p, "r", encoding="utf-8-sig").read()
io.open(p, "w", encoding="utf-8", newline="\n").write(raw)
print("mem_delta.py normalized")
