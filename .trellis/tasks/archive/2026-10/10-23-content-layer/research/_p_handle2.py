import io
path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\api\handle.rs"
with io.open(path, "r", encoding="utf-8") as f:
    c = f.read()

old = """        if w_phys == 0 || h_phys == 0 {
            self.clear_content();
            return;
        }
        if w_phys > 8192 || h_phys > 8192 {
            if self.shared.warn_content_oversize() {
                log::warn!(
                    "[content] update_content id={}: size {}x{} > 8192, discarded",
                    self.id,
                    w_phys,
                    h_phys
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
                    self.id,
                    buffer.len(),
                    expected
                );
            }
            return;
        }"""

new = """        // 校验决策在 core::content（0-unsafe 纯函数，AC2 单测矩阵盯着判定表）。
        match crate::core::content::plan_update(w_phys, h_phys, buffer.len()) {
            crate::core::content::ContentPlan::Clear => self.clear_content(),
            crate::core::content::ContentPlan::TooLarge => {
                if self.shared.warn_content_oversize() {
                    log::warn!(
                        "[content] update_content id={}: size {}x{} exceeds {}, discarded",
                        self.id,
                        w_phys,
                        h_phys,
                        crate::core::content::MAX_CONTENT_DIM
                    );
                }
            }
            crate::core::content::ContentPlan::LenMismatch => {
                if self.shared.warn_content_len() {
                    log::warn!(
                        "[content] update_content id={}: buffer len {} != w*h*4, discarded",
                        self.id,
                        buffer.len()
                    );
                }
            }
            crate::core::content::ContentPlan::Upload => {
                let copy: std::sync::Arc<[u8]> = buffer.to_vec().into_boxed_slice().into();
                self.shared.push(Command::UpdateContent {
                    id: self.id,
                    w_phys,
                    h_phys,
                    buffer: copy,
                    dirty,
                });
            }
        }"""

assert old in c, "old validation not found"
c = c.replace(old, new, 1)

# Remove the old tail (copy + push) that is now inside Upload arm
old_tail = """        let copy: std::sync::Arc<[u8]> = buffer.to_vec().into_boxed_slice().into();
        self.shared.push(Command::UpdateContent {
            id: self.id,
            w_phys,
            h_phys,
            buffer: copy,
            dirty,
        });
    }

    /// Clear content texture (release GPU memory)."""
new_tail = """    }

    /// Clear content texture (release GPU memory)."""
assert old_tail in c, "old tail not found"
c = c.replace(old_tail, new_tail, 1)

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("handle.rs: validation now delegates to core::content")
