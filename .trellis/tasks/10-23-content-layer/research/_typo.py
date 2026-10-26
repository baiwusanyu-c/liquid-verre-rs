import io, os
roots = [r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src",
         r"D:\github-project\liquid-verre-rs\crates\liquid-verre\shaders",
         r"D:\github-project\liquid-verre-rs\playground\src",
         r"D:\github-project\liquid-verre-rs\tools"]
hits = []
for root in roots:
    for dp, _, fs in os.walk(root):
        for fn in fs:
            p = os.path.join(dp, fn)
            try:
                t = io.open(p, encoding="utf-8").read()
            except Exception:
                continue
            if "''" in t:
                for line in t.splitlines():
                    if "''" in line:
                        hits.append((p, line.strip()[:100]))
for h in hits:
    print(h)
print("total:", len(hits))
