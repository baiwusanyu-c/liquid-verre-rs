import io
p = r"D:\github-project\liquid-verre-rs\.trellis\tasks\10-23-content-layer\research\mem_delta.py"
c = io.open(p, "r", encoding="utf-8").read()
old = 'ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))'
new = 'ROOT = r"D:\github-project\liquid-verre-rs"  # research-only tool; repo root pinned'
assert old in c
c = c.replace(old, new, 1)
io.open(p, "w", encoding="utf-8", newline="\n").write(c)
print("ROOT fixed")
