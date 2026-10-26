import io

path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\api\handle.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()

# Add AtomicBool to existing import
old = "use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};"
if old in content:
    print("AtomicBool already imported")
else:
    # Check current atomic imports
    old2 = "use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};"
    new2 = "use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};"
    if old2 in content:
        content = content.replace(old2, new2, 1)
        print("AtomicBool import added")
    else:
        print("Neither import pattern found, searching...")
        for i, line in enumerate(content.split('\n')[:30]):
            if 'atomic' in line.lower():
                print(f'{i+1}: {line}')

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("Done")
