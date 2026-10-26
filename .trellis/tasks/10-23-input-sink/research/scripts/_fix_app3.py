import io, os

path = os.path.join(os.getcwd(), "crates", "liquid-verre", "src", "runtime", "app.rs")
with io.open(path, "r", encoding="utf-8", newline="") as f:
    content = f.read()

# Fix: the DragSessionRef type alias should come before impl App
# Current wrong order: doc -> blank -> impl App -> pub(crate) type DragSessionRef
# Correct order: doc -> pub(crate) type DragSessionRef -> blank -> impl App

old = '''/// `App` 里持有的拖拽会话（独立类型别名只是为了签名短一点）。

impl App {
    #[inline]
    pub(crate) fn sink_for(&self, id: u32) -> Option<std::sync::Arc<dyn crate::api::input::GlassInputSink>> {
        self.block_sinks
            .iter()
            .find(|(bid, _)| *bid == id)
            .and_then(|(_, sr)| sr.0.clone())
    }
}
pub(crate) type DragSessionRef = crate::input::DragSession;'''

new = '''/// `App` 里持有的拖拽会话（独立类型别名只是为了签名短一点）。
pub(crate) type DragSessionRef = crate::input::DragSession;

impl App {
    #[inline]
    pub(crate) fn sink_for(&self, id: u32) -> Option<std::sync::Arc<dyn crate::api::input::GlassInputSink>> {
        self.block_sinks
            .iter()
            .find(|(bid, _)| *bid == id)
            .and_then(|(_, sr)| sr.0.clone())
    }
}'''

if old in content:
    content = content.replace(old, new)
    with io.open(path, "w", encoding="utf-8", newline="") as f:
        f.write(content)
    print("fixed")
else:
    print("old pattern not found, checking content...")
    # Print what's around the area
    idx = content.find("DragSessionRef")
    if idx >= 0:
        print(content[idx-50:idx+300])