# Design — P0-1 内容层

架构见父 design.md §1；本文件是落点细节。

- `runtime/commands.rs`：`UpdateContent{id, w, h, buffer: Arc<[u8]>, dirty}` / `ClearContent{id}`
  两变体尾部追加（与 input-sink 的 SetInputSink 变体互不插队）。buffer 由 api 侧一次拷贝成
  `Arc<[u8]>` 入队（setter 同步成本 = 一次 memcpy，微秒~毫秒级由宿主自担，文档写明）。
- `gfx/`：新文件 `gfx/content_layer.rs`（<250 行则单文件；unsafe 归 gfx）：
  `ContentStore { layers: HashMap<u32, ContentLayer> }`，`ContentLayer { tex, srv, w, h }`；
  尺寸不符⇒重建纹理；`upload(&[u8], dirty)` 用 staging 或按行 `UpdateSubresource`
  （B8G8R8A8 行距对齐坑：纹理行距与源行距都是 w·4，无 padding 问题，但子矩形上传走
  `D3D11_SUBRESOURCE_DATA`/Map+memcpy 行循环——按 d3d11-gotchas 核对后再定，倾向 Map(D3D11_MAP_WRITE)
  + 逐行拷贝，dirty 窄条时避免整面上传）。
- shader：`shaders/content_ps.hlsl`（premul 纹理直出 + 圆角掩码 alpha 乘）；VS 复用 border pass 的
  fullscreen-triangle 模式 + viewport/scissor 定块矩形。`ContentCb` = **c19**
  （块矩形物理像素 / 纹理尺寸 / effective_radius / 反算掩码参数）；新 sampler 槽（linear clamp）
  + 新 SRV 槽位登记进 gfx 资源布局注释。圆角掩码参数与玻璃 PS 同一来源（`core/block.rs::effective_radius`
  派生），shader 里不写第二半径公式（C5）。
- compositor 接线：Pass B 的 z 序循环里，每块玻璃绘制完成后立即尝试 `draw_content(block_id)`
  （无纹理=直接 return，零成本路径）；border 反馈 pass 位置不动⇒内容天然在其下（FR3）。
- 失效：命令消费时置「该块矩形 ∪ dirty 映射区」脏，走现有块级重绘通道（`request_render` +
  影响盒收口，C4）；**不改** influence box 公式（内容不参与鼠标光效，帧预算不变）。
- 混合状态：premultiplied alpha（SRC_ALPHA=ONE 语义），与玻璃 pass 的 blend 状态分离，
  用独立 OMState 或就地切换后恢复（照 gfx 现有状态管理模式，参考 dcomp.rs/swapchain.rs 纪律）。
- playground：`GLASS_CONTENT_DEMO` 只在 playground（不进库 env 例外清单）；checker 图案生成
  为确定性（尺寸/相位固定），取证可复现。
- `verify_content.py`：驱动 playground env（`GLASS_DEFAULT_BLOCK=1`、`GLASS_INTERNAL_SHOT_AT_MS=2500`
  稳态帧）+ BMP 采样断言；退出码与非 0 输出对齐现有 8 脚本风格。


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
