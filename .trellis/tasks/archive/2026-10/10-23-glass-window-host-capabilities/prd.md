# PRD — 库窗口即本体：宿主集成能力补齐（父任务）

> 基线：liquid-verre-rs 0.4.2（`a23cd99`）· 源文档：本目录 `capability-list.md`（ikun 宿主侧盘点，2026-10-23）
> 核对记录：本目录 `research/code-claims-check.md`（清单论断 vs 代码现状，已逐项验证）

## 1. 背景与目标

ikun「分类框」要从「宿主 Win32 窗口 + 库玻璃块」双层叠放，切换为**库窗口即本体**：
分类框的窗口本体直接由 `GlassWindow` 块承担，图标/栅格/文字/按钮/多选/拖出/右键菜单
全部由宿主在块上驱动。宿主实测结论：16 项分类框功能里 **8 项直接失效**，根因是 Win32
铁律（鼠标/键盘消息只发给 HWND）——画布每屏只有一个，块只是画布上的绘制矩形 + 库内部
两条交互状态机（拖/缩放）。缺三样东西：**内容挂载点、事件分发、焦点系统**。

目标：补齐这三样的最小版本（`update_content` / input sink / `set_focus_hwnd`）
+ 两项配套（拖入命中承诺验证、move filter），使宿主可完成重构。

## 2. 需求清单

- R1（P0-1 内容层）：`GlassWindow` 提供每块内容位图提交/替换/清除，层序=「本块玻璃之上、
  resize border 反馈之下」；静态内容零重绘成本；无内容块零开销。
- R2（P0-2 输入路由）：块内指针事件（Down/Up/Move/DoubleClick/RightDown/Wheel）同步路由给
  宿主 sink；宿主在 Down 返回 `Consumed` 则库不开移动会话、不 SetCapture（OLE 拖出天然可用）；
  resize 把手带优先级恒高于 sink。
- R3（P0-2 兼容）：未设置 sink 的块行为与 0.4.2 **完全一致**（默认按压块内=移动会话）。
- R4（P1-1 焦点路由）：`set_focus_hwnd`——块被按压且宿主消费（或点击抬层）时库对宿主窗口
  SetFocus，使键盘消息（ESC/Delete/F2）可达；画布保持 `WS_EX_NOACTIVATE` 不抢焦点。
- R5（P1-2 拖入承诺）：宿主把 `IDropTarget` 注册到 `canvas_handle()`，按块自行路由；库需
  **实测验证**「块区域内画布可被命中 + 消息持续被 pump」并写入 README 契约（逐帧
  `WS_EX_TRANSPARENT` 切换存在竞态风险，见 research 文档）。
- R6（P2 move filter）：移动/缩放会话内候选块矩形交宿主 filter 实时替换（框间对齐吸附）。
  可选，默认延后。
- R7（文档与示例）：README API 速查补齐 + playground 提供可取证 demo；面板代码不进库。
- R8（版本）：纯增量 API，向后兼容，按 minor（0.5.0）发布；CHANGELOG 记录。

## 3. 硬约束

- C1 分层边界：`api/` 是唯一公共面；`core/` 零 Win32 / 零 unsafe；unsafe 只进 platform/gfx/capture
  与 runtime 平台点位；sink/事件类型从 `api/` 导出。
- C2 写路径一律走 `Command` 队列（setter 异步、帧首消费、延迟 ≤1 帧）；同步回调只允许
  「读/决策」点位，采用与 `EventHandler`（api/event.rs）同一契约：微秒级返回、不阻塞、不重入。
- C3 cbuffer `c0..c18` 语义冻结，新增从 **c19** 起；border 独立叠加 pass 结构不动。
- C4 BASELINE.md 9 条不变量不破坏，尤其不变量 5：影响盒单一出口 `MonitorRt::influence_px()`，
  内容层 dirty 必须并入统一失效通道，禁止第二套框计算。
- C5 单一来源：块命中/把手/圆角走 `core/`（`manager.rs::hit_test/hit_resize`、`handle.rs`、
  `block.rs::effective_radius`）既有出口，禁止另写常量。
- C6 日志前缀即接口（RAW_TARGETS 等被阀门脚本依赖）；热路径禁格式化日志；周期告警 warn-once。
- C7 未经用户明确要求不得 git commit/push；到回滚点位时**提醒用户**发起临时提交。
- C8 中文文档一律 UTF-8 无 BOM + LF。

## 4. 第一门禁（用户钦定，压倒一切）

**玻璃视觉效果与性能永远第一。** 任何取舍与它冲突时：砍功能，不砍门禁。

1. 零差异原则：无内容层 + 未设置 sink 的全量构建，渲染输出与 0.4.2 基线**像素 0 差异**
   （`cmp_visual.py` 口径：single 布局四段 deep/interior/ring/outside 全 0）。
2. 每个子任务合入前：`verify_dirty` 连跑 ≥3 次、BASELINE 8 个回归脚本全绿、`gate_perf.py`
   通过（机器健康窗口 `rel_max < 5 ms`、`acq_max < 25 ms`；轮转 A/B ≥3 重复取中位数；
   取证期间禁止并发构建）。
3. 功能开启态的性能/视觉另立达标口径（见各子任务 AC），但不得反噬第 1/2 条。

## 5. 跨子任务验收标准

- A1 每个能力：公共 API + rustdoc（含线程契约与反例警示，如「sink 回调内禁 DoDragDrop」）
  + README 速查条目。
- A2 单测覆盖核心状态机（决策逻辑全部做成 `core/` 0-unsafe 可测）；不新增 `LiquidParams`
  字段（若被迫新增，同步 `playground/src/schema.rs::liquid_fields()`）。
- A3 新取证脚本进 `tools/`（命名与现有 8 脚本同一风格），并接入 xtask/scripts 检查路径。
- A4 宿主视角端到端样例文档：能驱动「内容推流 + 点击选中 + 拖出 + 右键菜单」最小闭环说明。
- A5 §4 门禁全绿 + CHANGELOG 进 `## [0.5.0-unreleased]` 段。

## 6. 子任务地图

| 子任务 | 覆盖需求 | 依赖 | 备注 |
|---|---|---|---|
| `10-23-input-sink` | R2/R3 | 无 | 交互骨架，**建议先行**；P1-1 依赖其 Down/Consumed 路径 |
| `10-23-content-layer` | R1 | 无 | 与 input-sink 可并行，但共改 `runtime/commands.rs`/`api/handle.rs`，并行时命令变体追加在尾部、谨防合并冲突 |
| `10-23-focus-drop-routing` | R4/R5 | input-sink 完成 | 轻量任务（PRD-only） |
| `10-23-move-filter` | R6 | 无（独立） | 可选，默认延后到 0.5.0 集成评审之后 |

建议顺序：input-sink → content-layer → focus-drop → 集成评审（父任务）→ move-filter（可选）。

## 7. 非目标（宿主侧职责，库不做）

图标命中表/多选/橡皮筋/右键菜单/DoDragDrop/D2D 内容渲染/`IDropTarget` 实现；
库内不做文本/矢量渲染（内容层只吃位图）；不改采集与折射管线本身。
