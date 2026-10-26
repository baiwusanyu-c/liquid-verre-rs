import io
p = r"D:\github-project\liquid-verre-rs\.trellis\tasks\10-23-content-layer\research\mem_delta.py"
c = io.open(p, "r", encoding="utf-8").read()
assert "_fields__" in c
c = c.replace("_fields__", "_fields_", 1)
io.open(p, "w", encoding="utf-8", newline="\n").write(c)
print("typo fixed")
