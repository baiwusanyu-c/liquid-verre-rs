import io

path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\runtime\mod.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()

old = "        style_cache: StyleCache::default(),\n        block_sinks: Vec::new(),\n        cmd_buf: Vec::new(),"
new = "        style_cache: StyleCache::default(),\n        block_sinks: Vec::new(),\n        content_store: ContentStore::default(),\n        cmd_buf: Vec::new(),"
assert old in content, "old not found"
content = content.replace(old, new, 1)
print("mod.rs: content_store init added")

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("Written")
