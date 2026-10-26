import io

# Patch 1: Command enum
path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\api\handle.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()

old_enum = "    SetInputSink {\n        id: u32,\n        sink: SinkRef,\n    },\n}"
new_enum = """    SetInputSink {
        id: u32,
        sink: SinkRef,
    },
    UpdateContent {
        id: u32,
        w_phys: u32,
        h_phys: u32,
        buffer: std::sync::Arc<[u8]>,
        dirty: Option<[u32; 4]>,
    },
    ClearContent {
        id: u32,
    },
}"""
assert old_enum in content, "old_enum not found"
content = content.replace(old_enum, new_enum, 1)
print("Patch 1 OK")

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("Written")
