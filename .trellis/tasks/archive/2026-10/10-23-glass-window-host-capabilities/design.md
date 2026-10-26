# Design — 库窗口即本体（父任务：总体架构）

各子任务 design.md 引用本文件对应节，只补本任务落点细节。事实核对见 `research/code-claims-check.md`。

## 0. 三个硬点（DOM 三件套的库侧对应）

| DOM 成立前提 | 库侧补齐 | 落点 |
|---|---|---|
| 图层挂载点 | 每块内容纹理，Pass B 玻璃之后叠加 | `gfx/compositor.rs` + `runtime/commands.rs`（content-layer） |
| 事件分发 | 画布 wnd_proc 补消息分支 + 块级 sink 同步路由 | `platform/window.rs` + `core/manager.rs` + `input/drag.rs`（input-sink） |
| 焦点系统 | `set_focus_hwnd` + Consumed/抬层路径 SetFocus | `runtime/` 平台点位 + `input/`（focus-drop） |

## 1. 内容层（P0-1）

- 命令：`Command::UpdateContent { id, w_phys, h_phys, buffer: Arc<[u8]>, dirty: Option<[u32;4]> }`
  与 `Command::ClearContent { id }`。buffer = 预乘 BGRA8、物理像素、行距 w*4；dirty = 块内物理
  像素半开区间。setter 侧拷贝入队，帧首消费 ⇒ 提交延迟 ≤1 帧（C2）。
- 资源：`gfx/` 内 `block_id -> ContentLayer`（`ID3D11Texture2D(B8G8R8A8_UNORM)` + SRV）；尺寸变化
  重建纹理；`ClearContent`/块销毁即释放（显存 = w·h·4/块，规模由宿主负责）。
- 上传：按 dirty 分块 `UpdateSubresource`；dirty=None 全量。校验失败（长度≠w·h·4、超尺寸上限）
  ⇒ 丢弃 + warn-once（`[warn]`），不返回 Err（C6 / 可恢复错误契约）。
- 绘制：z 序路径上**逐块交错**：本块玻璃画完立刻画本块内容（若有），保证「上层块玻璃盖住下层块
  内容」的正确叠放。scissor + 圆角掩码与本块玻璃同源（C5：`core/sdf.rs`/`block.rs::effective_radius`），
  内容被裁到相同形状。premultiplied alpha blend；纹理 **1:1 不拉伸**、贴块矩形左上角，超出裁剪、
  缺省留透明（resize 后旧内容语义 ⇒ 宿主靠 `clear_content` 避免残留）。
- 层序终态：玻璃 → 内容 → border 反馈（border 已是独立最后叠加 pass，天然在内容之上）。
- shader：新增独立小 PS（`shaders/content_ps.hlsl` 一类，模式仿 border pass）+ `ContentCb` = **c19**
  + 新 SRV 槽位 + linear clamp sampler；不碰 c0..c18（C3）。
- 失效：`UpdateContent` 触发的脏 = 块矩形 ∪ dirty 映射区，走既有「块移动触发重绘」同一通道
  （request_render + influence box 统一收口，C4）；无内容块/无内容帧零额外 pass、零开销（§4.1）。

## 2. 输入路由（P0-2）

- 公共类型（`api/input.rs`，经 `api/mod` 导出）：`PointerPhase/PointerButton/PointerEvent/
  InputDecision/GlassInputSink` + `GlassWindow::set_input_sink(Option<Arc<dyn GlassInputSink>>)`。
  `button` 支持 Left/Right（右键菜单需要 Down/Up 成对，故 Up{Right} 也转发）；中键本轮不做。
- 消息补齐（`platform/window.rs`）：`WM_LBUTTONDBLCLK / WM_RBUTTONDOWN / WM_RBUTTONUP / WM_MOUSEWHEEL`。
- Down 决策顺序（核心重排）：
  1. 命中 resize 把手带（`hit_resize`，含块外扩 grip）→ 库缩放会话 + hover border，**不询问 sink**；
  2. 命中块矩形（`hit_test` 取 z 序最高，C5）→ 询问该块 sink `Down`：
     `Consumed` → 记 press ownership=(block, consumed)，不开移动会话、**不 SetCapture**；
     `DragMove` → 移动会话（现行为）；`Ignored` → 库默认（= DragMove，保 R3 兼容）；
  3. 块无 sink → 完全现状。
- Press ownership 状态机：`Option<PressOwner{block, decision}>`；Down 记录决策，该按压序列内
  Move/Up 粘滞转发给 owner 块（Up 必回 owner）；未按压时 Move/DoubleClick/RightDown/Wheel 转发给
  光标所在最上层块。状态转移做成 `core/` 纯函数（0-unsafe 可测，A2），`platform/` 只拆包喂数据。
- 移动会话改造点：`input/drag.rs` 的会话开启时机从「LBUTTONDOWN 即开」移到决策之后。
- 同步契约（C2，rustdoc 写死）：渲染线程 wnd_proc 内执行；微秒级返回、不阻塞、不重入、
  **禁 DoDragDrop**（OLE 模态泵会卡死渲染线程，玻璃停绘；拖出锚点=宿主隐藏壳窗口，清单§四已如此分工）。
- 光标：`WM_SETCURSOR` 优先级 = 把手箭头 > `cursor_override` > 默认箭头。
- 存储：`set_input_sink` 走 Command 入队；sink 以 `Arc` 存 runtime 层，消息读路径零分配零格式化。

## 3. 焦点路由（P1-1）与拖入（P1-2）

- `GlassWindow::set_focus_hwnd(isize)`；触发点：① Down 且 sink 返回 Consumed；② 块被点击抬层。
  SetFocus 在渲染线程（wnd_proc 上下文）执行；画布保持 `WS_EX_NOACTIVATE`（`platform/level.rs`
  的 SWP_NOACTIVATE 纪律不变）。
- 键盘消息仍由宿主壳窗口接收，库不转发键盘。注意 `window.rs` 现存 ESC→Shutdown 死路径的处置
  （子任务 PRD 的开放问题）。
- P1-2 竞态：`WS_EX_TRANSPARENT` 逐帧切换（`window.rs` ~432-443 行）⇒ OLE 拖入 `WindowFromPoint`
  可能命中延迟一帧的透明态。验证 + 降级预案（子任务 focus-drop-routing）：若实测竞态，改为
  「块存在期间常关 TRANSPARENT，仅靠 `WM_NCHITTEST` 的 HTTRANSPARENT 穿透」——代码注释
  （window.rs:8-9）本就声明「WS_EX_TRANSPARENT 不是命中测试契约」，此降级有既有依据，需重跑
  鼠标隔离回归。

## 4. Move filter（P2）

- `set_move_filter(Option<Arc<dyn Fn([f32;4])->[f32;4] + Send + Sync>>)`；会话每帧应用候选块矩形
  （逻辑像素）前过 filter，再走内置 clamp；返回值 sanitize（clamp_finite）后使用。None=零开销现状。

## 5. 兼容、性能与回滚

- 全部增量式（新命令变体/新 cbuffer c19+/新 api 模块）；`api/` 与 `core/` 保持 0 unsafe。
- 性能预算：静态内容帧 = 每可见内容块一次 draw（无上传）；拖拽帧 = 玻璃既有成本 + 常数小增量，
  由 gate_perf 把关。热路径（Move 转发）零分配。
- 回滚粒度=子任务；功能与 §4 门禁冲突时砍功能、留分支禁用开关位（不加对外 env，走代码回滚）。
- 版本 0.5.0 minor；发布不在本任务集（走 `scripts/release.mjs` 人工流程）。
