import io

path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\mod.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()

old = "pub(crate) mod compositor;\npub(crate) mod dcomp;\npub(crate) mod device;\npub(crate) mod glass_renderer;\npub(crate) mod gpu_timer;\npub(crate) mod pyramid;\npub(crate) mod shaders;\npub(crate) mod swapchain;"
new = "pub(crate) mod compositor;\npub(crate) mod content_layer;\npub(crate) mod dcomp;\npub(crate) mod device;\npub(crate) mod glass_renderer;\npub(crate) mod gpu_timer;\npub(crate) mod pyramid;\npub(crate) mod shaders;\npub(crate) mod swapchain;"

assert old in content, "mod.rs old not found"
content = content.replace(old, new, 1)

old_use = "pub(crate) use compositor::Compositor;\npub(crate) use dcomp::DComp;\npub(crate) use device::Device;\npub(crate) use glass_renderer::GlassRenderer;"
new_use = "pub(crate) use compositor::Compositor;\npub(crate) use content_layer::ContentStore;\npub(crate) use dcomp::DComp;\npub(crate) use device::Device;\npub(crate) use glass_renderer::GlassRenderer;"
assert old_use in content, "mod.rs old_use not found"
content = content.replace(old_use, new_use, 1)

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("mod.rs updated")
