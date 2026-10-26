import io
path = r"D:\github-project\liquid-verre-rs\playground\src\main.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()
old = 'if mode != "off" && mode != "0" && mode != "" {'
new = 'if mode != "off" && mode != "0" && !mode.is_empty() {'
assert old in content, "not found"
content = content.replace(old, new, 1)
with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("Fixed")
