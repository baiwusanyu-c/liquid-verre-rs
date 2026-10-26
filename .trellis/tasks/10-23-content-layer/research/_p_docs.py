import io
p = r"D:\github-project\liquid-verre-rs\.trellis\tasks\10-23-content-layer\design.md"
c = io.open(p, encoding="utf-8").read()
add = """

---

## 偏离登记（2026-10-26 回炉后终态）

1. **批量→交错：已改回原设计**。round-1 曾以「批量 pass 视觉等价」为由用
   draw_blocks→draw_contents→draw_borders 三段结构，验收指出这在重叠区不等价
   （下层内容会浮到上层玻璃之上）。终态为**真逐块交错**：z 序循环内每块玻璃
   Draw 后立即画该块内容（store 无此块⇒零成本跳过；空 store⇒连 HashMap 查找
   都没有），border 保持最后整体叠加。判别力实证见 tools/verify_content.py 头部
   OVERLAP_DATA（批量 25/25 裸纹素 vs 交错 0/25）。
2. **ContentCb 槽位（保留的偏离）**：设计写「c19」，实现用**独立 cbuffer b1**
   （玻璃只用 b0，border 用独立 b0；内容用 b1 后交错切换时**无需重绑玻璃 CB**，
   切管线只剩 2 次 SetShader + 1 次 SetSRV + 1 次 scissor）。c0..c18 冻结不动。
   内容纹理占 **t9**（玻璃只用 t0..t8），PS 用整数 Load，不占采样器。
3. **上传路径（保留的偏离）**：设计倾向 Map+逐行拷贝；实现用
   **D3D11_USAGE_DEFAULT + UpdateSubresource(带 box)**。理由：DYNAMIC 资源官方
   禁 plain MAP_WRITE、UpdateSubresource 不支持 DYNAMIC，两者都绕不开；DEFAULT
   纹理 GPU 常驻，PS 读取无 CPU/GPU 同步语义坑，dirty 子区走 box 上传天然免
   RowPitch 处理。踩坑记录：UpdateSubresource 带 pDstBox 时 pSrcData 必须指向
   **子矩形左上角**而非整图左上角（verify_content 断言 4 抓到，已修并注释）。
4. **失效通道修正**：UpdateContent 不再置 region_dirty（内容不改窗口区域）；
   ClearContent 消费返回 true（清除改变画面必须触发重绘帧）——round-1 两处
   行为 bug，回炉时修复。
"""
if "偏离登记（2026-10-26 回炉后终态）" not in c:
    c = c.rstrip() + "\n" + add
io.open(p, "w", encoding="utf-8", newline="\n").write(c)
print("design.md updated")

p2 = r"D:\github-project\liquid-verre-rs\.trellis\tasks\10-23-content-layer\implement.md"
c2 = io.open(p2, encoding="utf-8").read()
for item in ["1. 命令变体", "2. `gfx/content_layer.rs`", "3. shader", "4. 失效通道接线",
             "5. 单测", "6. `tools/verify_content.py`", "7. 门禁", "8. rustdoc"]:
    c2 = c2.replace("- [ ] " + item, "- [x] " + item) if ("- [ ] " + item) in c2 else c2
io.open(p2, "w", encoding="utf-8", newline="\n").write(c2)
print("implement.md checked")
