import io

path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\api\handle.rs"
with io.open(path, "r", encoding="utf-8") as f:
    content = f.read()

old = """        self.shared.push(Command::SetInputSink {
            id: self.id,
            sink: SinkRef(sink),
        });
    }

    /// """

new = """        self.shared.push(Command::SetInputSink {
            id: self.id,
            sink: SinkRef(sink),
        });
    }

    /// Update content texture (premultiplied BGRA8, physical pixels).
    ///
    /// `buffer` row pitch = `w_phys * 4` (no padding); `dirty` is a half-open
    /// interval `[left, top, right, bottom)` in block-local physical pixels,
    /// `None` = full upload.
    ///
    /// Validation failures (wrong length, size > 8192) are discarded with a
    /// warn-once log; no `Err` is returned. Size 0 is equivalent to
    /// [`clear_content`](Self::clear_content).
    ///
    /// This method does a single `memcpy` (`buffer` -> `Arc<[u8]>`); the actual
    /// texture upload happens on the render thread at frame start, latency <= 1 frame.
    pub fn update_content(
        &self,
        w_phys: u32,
        h_phys: u32,
        buffer: &[u8],
        dirty: Option<[u32; 4]>,
    ) {
        if w_phys == 0 || h_phys == 0 {
            self.clear_content();
            return;
        }
        if w_phys > 8192 || h_phys > 8192 {
            if self.shared.warn_content_oversize() {
                log::warn!(
                    "[content] update_content id={}: size {}x{} > 8192, discarded",
                    self.id, w_phys, h_phys
                );
            }
            return;
        }
        let expected = (w_phys as usize)
            .checked_mul(h_phys as usize)
            .and_then(|n| n.checked_mul(4));
        if expected != Some(buffer.len()) {
            if self.shared.warn_content_len() {
                log::warn!(
                    "[content] update_content id={}: buffer len {} != w*h*4 = {:?}, discarded",
                    self.id, buffer.len(), expected
                );
            }
            return;
        }
        let copy: std::sync::Arc<[u8]> = buffer.to_vec().into_boxed_slice().into();
        self.shared.push(Command::UpdateContent {
            id: self.id,
            w_phys,
            h_phys,
            buffer: copy,
            dirty,
        });
    }

    /// Clear content texture (release GPU memory).
    ///
    /// Call before hiding, recycling, or resizing. Repeated calls or calls on
    /// blocks without content are harmless.
    pub fn clear_content(&self) {
        self.shared.push(Command::ClearContent { id: self.id });
    }

    /// """

assert old in content, "old block not found"
content = content.replace(old, new, 1)
print("Patch 2 matched and replaced")

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(content)
print("Written")
