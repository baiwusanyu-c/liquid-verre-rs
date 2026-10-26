import io

gp = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\glass_renderer.rs"
with io.open(gp, "r", encoding="utf-8") as f:
    c = f.read()

start = c.find("    /// Draw content layer overlays on top of glass")
assert start >= 0, "draw_contents doc not found"
# The method ends right before the final two closes of the impl block; find its terminating
# "    }\n}" — it is the LAST method in the file.
tail = c[start:]
# Save the removed batch method for the probe patch.
with io.open(r"D:\github-project\liquid-verre-rs\.trellis\tasks\10-23-content-layer\research\batch_draw_contents.snippet", "w", encoding="utf-8", newline="\n") as f:
    f.write(tail)
# cut: drop method body, keep the trailing "}\n" that closed the impl block
end = tail.find("\n}\n")  # impl block close after method close
assert end >= 0, "impl close not found"
new_c = c[:start] + tail[end+1:]
assert new_c.endswith("}\n"), repr(new_c[-50:])
with io.open(gp, "w", encoding="utf-8", newline="\n") as f:
    f.write(new_c)
print("draw_contents batch method removed (archived for probe)")

# compositor: pass store into draw_blocks, drop batch call
cp = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\compositor.rs"
with io.open(cp, "r", encoding="utf-8") as f:
    cc = f.read()

old_call = """                time,
                press,
                hover_id,
            );
            // 缩放反馈 border"""
assert old_call in cc, "draw_blocks call tail not found"
new_call = """                time,
                press,
                hover_id,
                Some(content_store),
            );
            // 缩放反馈 border"""
cc = cc.replace(old_call, new_call, 1)

old_batch = """            glass.draw_contents(
                &gfx.ctx,
                &crate::gfx::glass_renderer::BorderTarget {
                    rtv: &self.rt.rtv,
                    w: self.rt.w,
                    h: self.rt.h,
                    canvas: [self.width as f32, self.height as f32],
                },
                blocks,
                order,
                content_store,
            );
"""
assert old_batch in cc, "batch call not found"
cc = cc.replace(old_batch, "", 1)

with io.open(cp, "w", encoding="utf-8", newline="\n") as f:
    f.write(cc)
print("compositor wired to interleaved draw_blocks")
