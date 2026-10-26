# PRD — P0-1 内容层 update_content/clear_content

> 前置阅读：父任务 prd.md（§3 约束、§4 第一门禁）与 design.md §1。

## 依赖与顺序

- 无状态机依赖，可与 input-sink 并行；共改 `runtime/commands.rs`/`api/handle.rs`，
  命令变体追加在枚举尾部，谨防互踩（并行实施时不得回滚对方段落）。
- 不依赖 sink；宿主可在本任务落地后先用「静态内容 + 库默认交互」跑通渲染链路。

## 需求

- FR1 `GlassWindow::update_content(&self, w_phys: u32, h_phys: u32, buffer: &[u8],
  dirty: Option<[u32;4]>)`：预乘 BGRA8、物理像素、行距 = w·4；dirty = 块内物理像素半开区间，
  None = 全量。异步入队、帧首消费（延迟 ≤1 帧，C2）。
- FR2 `GlassWindow::clear_content(&self)`：清除本块内容并释放纹理（隐藏/回收/resize 前调用）。
- FR3 层序：本块玻璃之上、resize border 反馈之下；z 序逐块交错绘制（上层块玻璃盖下层块内容）。
- FR4 内容被本块有效圆角掩码裁剪（与玻璃同形状，C5 单一来源）；纹理 1:1 不缩放，贴块矩形左上角，
  超出裁剪、缺省透明。
- FR5 成本：同内容不重传；dirty 只上传子区；**无内容块/无内容帧零开销**（不产生额外 pass/绘制/
  分配——这是父 §4.1 零差异门禁的实现前提）。
- FR6 健壮性：buffer 长度 ≠ w·h·4、尺寸超上限（定 8192×8192 或按显存预算）⇒ 丢弃该条 +
  warn-once（`[warn]`），不返回 Err（可恢复错误契约）；NaN 无关（纯位图），尺寸 0 ⇒ 等价 clear。
- FR7 失效统一：内容更新触发的重绘走既有 influence box/dirty 同一收口（C4），不新增第二套框。
- FR8 playground 取证入口：新增 env 开关（`GLASS_CONTENT_DEMO=off|checker|solid`，仅构造期读一次）
  生成确定性图案内容块，供取证脚本驱动。

## 验收标准

- AC1 第一门禁（压倒一切）：`GLASS_CONTENT_DEMO=off` 全量构建 vs 0.4.2 基线**像素 0 差异**
  （single 四段全 0；many 按跨版本圆角口径）；`verify_dirty` 连跑 ≥3；8 回归脚本全绿；
  `gate_perf` 通过（无内容帧的 vsync 预算与基线同档）。
- AC2 单测（0-unsafe 侧）：命令消费、参数校验矩阵（长度/上限/0 尺寸）、clear 状态机、
  dirty 区间裁剪计算。
- AC3 新取证 `tools/verify_content.py`（只看 `GLASS_INTERNAL_SHOT` composed 图）：
  ① checker 图案中心/四角/圆角带/块外采样点数值符合预期（圆角外无内容渗出）；
  ② 上层块玻璃盖住下层块内容（叠放语义）；③ clear 后区域回到纯玻璃；
  ④ dirty 局部更新时**仅**目标区像素变化（差异计数）；⑤ resize 块 + clear + 新尺寸推流无拉伸残留。
- AC4 性能与显存：`ab_memory` 通道可读 Δ显存 ≈ Σw·h·4；内容块存在时拖拽帧不破 gate_perf 门槛。
- AC5 rustdoc + README API 速查 + CHANGELOG；不新增 `LiquidParams` 字段（schema 单测不破）。
