import io

path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\api\handle.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()

# Add fields after fatal
old_fields = "    fatal: Mutex<Option<Error>>,\n}"
new_fields = "    fatal: Mutex<Option<Error>>,\n    warn_oversize: AtomicBool,\n    warn_len: AtomicBool,\n}"
assert old_fields in content, "old_fields not found"
content = content.replace(old_fields, new_fields, 1)
print("Patch 3a: Shared fields added")

# Add initialization after fatal init
old_init = "            fatal: Mutex::new(None),\n        })"
new_init = "            fatal: Mutex::new(None),\n            warn_oversize: AtomicBool::new(false),\n            warn_len: AtomicBool::new(false),\n        })"
assert old_init in content, "old_init not found"
content = content.replace(old_init, new_init, 1)
print("Patch 3b: Shared init added")

# Add warn_once methods after mark_stopped
old_mark = "    pub(crate) fn mark_stopped(&self) {\n        self.stopped.store(true, Ordering::Release);\n    }"
new_mark = """    pub(crate) fn mark_stopped(&self) {
        self.stopped.store(true, Ordering::Release);
    }

    /// warn-once gate: content size > 8192.
    /// Returns true the first time (caller should log), false thereafter.
    pub(crate) fn warn_content_oversize(&self) -> bool {
        !self.warn_oversize.swap(true, Ordering::Relaxed)
    }

    /// warn-once gate: content buffer length mismatch.
    /// Returns true the first time (caller should log), false thereafter.
    pub(crate) fn warn_content_len(&self) -> bool {
        !self.warn_len.swap(true, Ordering::Relaxed)
    }"""
assert old_mark in content, "old_mark not found"
content = content.replace(old_mark, new_mark, 1)
print("Patch 3c: warn methods added")

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("Written")
