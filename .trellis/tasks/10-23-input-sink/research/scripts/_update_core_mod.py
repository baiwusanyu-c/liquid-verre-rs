import io, os

path = os.path.join(os.getcwd(), "crates", "liquid-verre", "src", "core", "mod.rs")

with io.open(path, "r", encoding="utf-8", newline="") as f:
    content = f.read()

old = """pub(crate) mod manager;
pub(crate) mod params;
pub(crate) mod quality;
pub(crate) mod sdf;
pub(crate) mod style;"""

new = """pub(crate) mod manager;
pub(crate) mod params;
pub(crate) mod quality;
pub(crate) mod routing;
pub(crate) mod sdf;
pub(crate) mod style;"""

content = content.replace(old, new)

with io.open(path, "w", encoding="utf-8", newline="") as f:
    f.write(content)

print("core/mod.rs updated OK")