import io

path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\content_layer.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()

# Add manual Debug impls
# Find the end of ContentLayer struct
old_cl = "pub(crate) struct ContentLayer {\n    pub tex: ID3D11Texture2D,\n    pub srv: ID3D11ShaderResourceView,\n    pub w: u32,\n    pub h: u32,\n}"
new_cl = """pub(crate) struct ContentLayer {
    pub tex: ID3D11Texture2D,
    pub srv: ID3D11ShaderResourceView,
    pub w: u32,
    pub h: u32,
}

impl std::fmt::Debug for ContentLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContentLayer")
            .field("w", &self.w)
            .field("h", &self.h)
            .finish()
    }
}"""
assert old_cl in content, "ContentLayer struct not found"
content = content.replace(old_cl, new_cl, 1)
print("ContentLayer Debug added")

# Add Debug for ContentStore
old_cs = "#[derive(Default)]\npub(crate) struct ContentStore {\n    layers: HashMap<u32, ContentLayer>,\n}"
new_cs = """#[derive(Default)]
pub(crate) struct ContentStore {
    layers: HashMap<u32, ContentLayer>,
}

impl std::fmt::Debug for ContentStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContentStore")
            .field("layers", &self.layers.len())
            .finish()
    }
}"""
assert old_cs in content, "ContentStore struct not found"
content = content.replace(old_cs, new_cs, 1)
print("ContentStore Debug added")

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("Written")
