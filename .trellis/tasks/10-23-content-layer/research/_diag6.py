import io, os
p = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\glass_renderer.rs"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = """        let store = contents.filter(|s| !s.is_empty());
        if store.is_some() {"""
new = """        let store = contents.filter(|s| !s.is_empty());
        {
            static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n % 60 == 0 {
                log::info!(
                    target: "verify",
                    "DIAG2 draw_blocks call={} contents_some={} store_nonempty={} blocks={} order={}",
                    n,
                    contents.is_some(),
                    store.is_some(),
                    blocks.len(),
                    order.len()
                );
            }
        }
        if store.is_some() {"""
assert old in c
c = c.replace(old, new, 1)
old2 = """            if let Some(store) = store {
                if let Some(layer) = store.get(block.id) {"""
new2 = """            if let Some(store) = store {
                if store.get(block.id).is_some() && DIAG3.swap(true, std::sync::atomic::Ordering::Relaxed) == false {
                    log::info!(target: "verify", "DIAG3 first content hit id={}", block.id);
                }
                if let Some(layer) = store.get(block.id) {"""
assert old2 in c
c = c.replace(old2, new2, 1)
old3 = "    pub fn draw_blocks("
new3 = "    static DIAG3: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);\n\n    pub fn draw_blocks("
# put the static at module level instead: insert before `impl GlassRenderer {` second occurrence? simpler: module-level static after CB consts
idx = c.find("\n#[repr(C)]\n#[derive(Clone, Copy)]\nstruct ContentCb")
assert idx > 0
c = c[:idx] + "\nstatic DIAG3: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);\n" + c[idx:]
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("DIAG2/3 added")
