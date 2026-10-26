import io

path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\runtime\commands.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()

old = "                let mi = b.monitor();"
new = "                let mi = b.monitor;"
assert old in content, "not found"
content = content.replace(old, new, 1)
print("Fixed: monitor() -> monitor")

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("Written")
