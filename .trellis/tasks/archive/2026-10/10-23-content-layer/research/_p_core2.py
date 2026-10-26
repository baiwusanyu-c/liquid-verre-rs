import io
path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\core\mod.rs"
with io.open(path, "r", encoding="utf-8") as f:
    c = f.read()
old = "pub(crate) mod block;\npub(crate) mod draw;"
new = "pub(crate) mod block;\npub(crate) mod content;\npub(crate) mod draw;"
assert old in c, "not found"
c = c.replace(old, new, 1)
with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("core/mod.rs: content module registered")
