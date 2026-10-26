import io
p = r"D:\github-project\liquid-verre-rs\.trellis\tasks\10-23-content-layer\research\mem_delta.py"
c = io.open(p, "r", encoding="utf-8").read()
old = """    log = io.open(os.path.join(ROOT, ".trellis", "tasks", "10-23-content-layer",
                               "research", "memdelta_%s.log" % tag), "wb")"""
new = """    HERE = os.path.dirname(os.path.abspath(__file__))
    log = io.open(os.path.join(HERE, "memdelta_%s.log" % tag), "wb")"""
assert old in c
c = c.replace(old, new, 1)
io.open(p, "w", encoding="utf-8", newline="\n").write(c)
print("path fixed")
