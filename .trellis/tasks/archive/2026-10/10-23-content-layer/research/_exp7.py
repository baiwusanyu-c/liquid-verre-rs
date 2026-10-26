import io, os
p = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\glass_renderer.rs"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
# in draw_content_for_block, swap content shaders for BORDER shaders (which are known-good)
old = """            ctx.VSSetShader(&self.cvs, None);
            ctx.PSSetShader(&self.cps, None);
            ctx.Draw(6, 0);"""
new = """            ctx.VSSetShader(&self.cvs, None);
            ctx.PSSetShader(&self.cps, None);
            ctx.Draw(6, 0);
            // EXPERIMENT v3: also draw with the known-good border shaders
            ctx.VSSetShader(&self.bvs, None);
            ctx.PSSetShader(&self.bps, None);
            ctx.Draw(6, 0);"""
assert old in c
c = c.replace(old, new, 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("exp v3: border-shader control draw added")
