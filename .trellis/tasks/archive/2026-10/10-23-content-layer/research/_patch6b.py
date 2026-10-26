import io

path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\runtime\app.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()

# Match using just the field definition + next line start
old_field = "    pub block_sinks: Vec<(u32, SinkRef)>,\n"
new_field = "    pub block_sinks: Vec<(u32, SinkRef)>,\n    pub content_store: ContentStore,\n"
assert old_field in content, "field not found"
content = content.replace(old_field, new_field, 1)
print("app.rs: content_store field added")

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("Written")
