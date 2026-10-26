# PRD — P0-2 输入路由 GlassInputSink

> 前置阅读：父任务 `10-23-glass-window-host-capabilities` 的 prd.md（§3 约束、§4 第一门禁）
> 与 design.md §2。本文件只列本任务特有需求与验收。

## 依赖与顺序

- 无前置，**建议本任务树中第一个 start**（P1 焦点路由依赖其 Down/Consumed 路径）。
- 与 content-layer 并行时共改 `runtime/commands.rs`/`api/handle.rs`：命令变体只追加在枚举尾部。

## 需求

- FR1 公共面（进 `api/` 导出，父 design §2 类型清单）：`PointerPhase`（含 Down/Up/Move/DoubleClick/
  RightDown/Wheel，Up 可携带右键）、`PointerButton{Left,Right}`、`PointerEvent{block_local:[f32;2],
  phase, button, wheel_delta}`、`InputDecision{Consumed,DragMove,Ignored}`、
  `trait GlassInputSink: Send+Sync { on_pointer(&self, id:u32, &PointerEvent)->InputDecision;
  cursor_override(&self, id:u32)->Option<isize> }`、`GlassWindow::set_input_sink(Option<Arc<dyn ...>>)`。
  事件覆盖宿主全部用例：图标点击/选中/Ctrl-Shift 多选、按住拖出、空白框选（Down+Move 连拍）、
  双击打开、右键菜单（Down/Up 成对）、滚轮、hover+光标覆盖。中键本轮不做。
- FR2 画布消息补齐（`platform/window.rs`）：`WM_LBUTTONDBLCLK / WM_RBUTTONDOWN / WM_RBUTTONUP /
  WM_MOUSEWHEEL`；Wheel 的 `wheel_delta = delta/120`（一格 ±1）。
- FR3 决策顺序硬规则：把手带（含块外扩 grip，走 `core/manager.rs::hit_resize`）优先级恒高于 sink，
  命中即库缩放会话 + hover border，**不询问 sink**；块矩形内（`hit_test` z 序最高）询问 sink。
- FR4 `Consumed` = 本次按压不开移动会话、**不 SetCapture**（OLE 拖出天然可用的前提）；
  `DragMove` = 移动会话；`Ignored` = 库默认（= DragMove，保持无 sink 现状）。
- FR5 粘滞转发：Down 后的同一按压序列 Move/Up 绑定 owner 块（Up 必回 owner，即使光标已离开该块）；
  未按压时 Move/DoubleClick/RightDown/Wheel 转发光标所在最上层块。
- FR6 兼容（R3）：sink=None 的块行为与 0.4.2 完全一致；未设置任何 sink 的构建，鼠标行为/像素
  与基线零差异（`verify_mouse_isolation`、拖拽/缩放语义原样）。
- FR7 光标：`WM_SETCURSOR` 优先级 = 把手箭头 > cursor_override > 默认箭头。
- FR8 契约（rustdoc 写死）：sink 在渲染线程 wnd_proc 上下文**同步**执行；微秒级返回、不阻塞、
  不重入引擎（与 EventHandler 同契约）；**回调内禁 DoDragDrop / 禁等待任何库的阻塞原语**
  （OLE 模态泵会卡死渲染线程，拖出用宿主壳窗口锚点）。回调内调库 setter（如 set_input_sink 换
  决策、抬层）允许——setter 是异步入队；入队语义需在文档写明「≤1 帧后生效」。
- FR9 playground 提供 sink demo 块（事件日志 + 可配置「某区域返回 Consumed」的演示），供取证脚本驱动。

## 验收标准

- AC1 第一门禁（父 §4）：无 sink 全量构建像素 0 差异（single 四段全 0）+ 8 回归脚本全绿 +
  `verify_dirty` 连跑 ≥3 + `gate_perf` 通过。
- AC2 `core/` 0-unsafe 单测：决策状态机全矩阵（把手/块内/块外 × 左/右/滚轮 × 有无 sink ×
  Consumed/DragMove/Ignored × 粘滞与 Up 回 owner × z 序变化时的归属）。
- AC3 新取证 `tools/verify_input_sink.py`（`GLASS_CURSOR_AT` 钉光标 + 显式 `GLASS_DEFAULT_BLOCK=1`）
  断言：① demo 块返回 Consumed 的区域内按压拖动 → **块不动**；② 其余区域 DragMove → 块动；
  ③ RightDown/RightUp/DoubleClick/Wheel 到达 sink（`[verify]` 前缀日志取证）；④ sink=Consumed 时
  把手带仍然缩放（优先级规则）；⑤ Consumed 按压期间 `GetCapture` 归宿主/为空而非画布（拖出兼容）。
- AC4 rustdoc + README 线程模型/API 速查更新 + CHANGELOG `[0.5.0-unreleased]`。
- AC5 clippy/fmt/`node scripts/check.mjs` 全绿（xtask 与 scripts 两条路径等价）。
