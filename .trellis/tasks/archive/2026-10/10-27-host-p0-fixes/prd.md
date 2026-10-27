# PRD: 0.5.0 宿主实测四缺陷修复（#1 画布重建杀线程 / #2 类注册容忍 / #3 点击穿透自锁 / #4 DPI 重估）

## Background

宿主（ikun 分类框）在 crates.io `liquid-verre-rs 0.5.0` 真实环境复现并定位了四条缺陷，
报告已存档于 `research/defect-fix-plan-host.md`（下称"附件"）。四条缺陷的宿主侧止血
（`wake_canvas_idle`、`revive_if_dead + UnregisterClassW`）不在本任务范围——那是宿主仓库的工作。

优先级（附件结论）：#3 交互全灭感知最强 → #1+#2 成对出现（热插拔后永久降级）→ #4 撕裂/错位。

## Requirements

### R1（缺陷 #1，P0）rebuild 销毁旧画布不得终结引擎

- 现象：显示器变化 → `rebuild_monitors()` 对旧画布 `DestroyWindow` → `WM_DESTROY`
  无条件 `PostQuitMessage(0)` → 渲染线程消息循环退出 → emit `Shutdown` → 全部玻璃消失，
  宿主永久退回旧渲染。
- 要求：「画布销毁」与「引擎退出」分离。只有引擎整体退出路径允许终止消息循环。
- 验收 AC1：向画布发 `WM_DISPLAYCHANGE`/`WM_DPICHANGED` 触发 rebuild 后，引擎存活——
  `[metrics]` 帧日志继续、`MonitorsChanged` 事件发出、无 `Shutdown` 事件、块仍可交互渲染。
  取证脚本 `tools/verify_monitors_rebuild.py`（判别力：删实现必红）。

### R2（缺陷 #2，P0）register_class 必须真正容忍"类已存在"

- 现象：同进程第二个 `Engine` 重建时 `RegisterClassExW` 失败 `0x80070582`（ERROR_CLASS_ALREADY_EXISTS
  的 HRESULT 形态），代码注释声称容忍但比较写错，分支永不命中。
- 要求：`ERROR_CLASS_ALREADY_EXISTS(1410)` 走容忍分支；其余错误码仍报错。
- 验收 AC2：
  - 单测钉死比较逻辑本身（`0x80070582` 命中容忍；`0x80070005` 等不命中）。
  - 同进程 `Engine::new + start + shutdown` 两轮不报 `[window-creation]`（集成级验证，
    受真实桌面依赖约束，按项目纪律以本地脚本/手工轮次记录，不进 CI）。

### R3（缺陷 #3，P0）遮挡停产后点击穿透状态机必须自动恢复

- 现象：不透明窗口盖满玻璃块 → 库日志「桌面采集停产」→ 移开窗口后块可见，但画布
  `WS_EX_TRANSPARENT` 永不解除（宿主直读 exstyle 实测 900ms+ 仍带），全部鼠标交互死亡，
  不自愈；宿主 `PostMessage` 唤醒后穿透立即解除、sink 点击恢复（坐实"没人推进状态机"）。
  独占全屏让路路径同理。
- 要求：消息循环存在**不依赖采集帧与鼠标消息**的兜底驱动源（定时器级），保证
  光标轮询 + 穿透状态机 + 遮挡复检在停产期间仍被周期性推进。
- 验收 AC3：取证脚本 `tools/verify_clickthrough_recovery.py`：盖块 → 等停产日志 →
  移开 → `SetCursorPos` 进块 → 断言 300 ms 内 `GWL_EXSTYLE & WS_EX_TRANSPARENT == 0`，
  且后续物理点击拖动生效（块移动）。脚本带判别力设计（反向对照：光标停在块外时
  穿透必须仍在——防止"恒不穿透"假绿）。

### R4（缺陷 #4，P1）MonitorsChanged 后块几何按新 DPI 重估，保持逻辑尺寸

- 现象：200%↔100% 切换（或主屏变更）后块滞留旧坐标空间：画在屏外/半屏错位、
  与宿主内容窗口撕裂；宿主上午的"块尺寸 2x 膨胀"即此表现。
- 定界修正（本地代码核对结论，详见 design.md）：附件提议的"逐块 sync"在
  `rebuild_monitors()` 里**已存在**；真正的根因是 ① sync 前把 `b.scale` 强清 1.0，
  摧毁了新旧比例换算的基准（k=new_scale/1.0 ⇒ 物理尺寸被重乘新缩放）；
  ② `monitor_rects` 刷新发生在 rebuild **之后**，块归屏判定用的是旧屏矩形。
- 要求：rebuild 重估保持「逻辑尺寸不变」语义（1000px@200% 的块换到 100% 屏应为
  物理 500px）；归屏判定基于**新**显示器矩形；因分辨率变化越界的块被拉回
  「中心不出屏」合法区。
- 验收 AC4：
  - core 层纯函数单测：200→100 尺寸减半（不膨胀回归）、100→200 尺寸加倍、
    同屏同缩放不动、跨屏归属、越界钳制、显示器数量缩减后索引仍合法。
  - `tools/verify_monitors_rebuild.py` 断言 rebuild 后块矩形在屏内且快照立即发布。

## Constraints（项目红线）

- 不破坏 BASELINE.md 9 条不变量（本任务不触碰 blur kernel / scissor / cbuffer / 影响盒）。
- 分层边界：新增纯计算进 `core/`（0-unsafe、可单测）；`unsafe` 仍集中在 `platform/` 等既有位置；
  `api/` 公共面零变更（不新增 `LiquidParams` 字段，schema.rs 无需动）。
- 修复全部落在 runtime/platform 内部与 core 纯函数，不开新的上层旁路。
- 日志前缀接口不动（`RAW_TARGETS` 原样）；热路径禁止格式化日志；新定时器回调零分配。
- 提交前五连全绿：`cargo test --release --workspace` / `clippy -D warnings` / `fmt --check` /
  `node scripts/check.mjs` / `python tools/gate_perf.py`；并复跑受影响的 BASELINE 回归脚本。
- 未经用户明确要求不得 git commit。

## Non-Goals

- 不清理宿主侧止血代码（宿主仓库的工作）。
- 不把消息泵重构为 `MsgWaitForMultipleObjectsEx`（附件列为等价替代，择定时器方案）。
- 不引入 `SetWinEventHook` 事件驱动遮挡检测（遮挡模块既有 Non-Goals）。
- 不改跨版本视觉基线（BASELINE.md 取证图口径不变）。
- 不做 0.5.1 发版（发版由用户另行发起 `release.mjs`）。

## Open Question（诚实记录）

附件对 #3 的停摆机制描述为"循环阻塞在 GetMessage"，但本地 0.5.0 主循环是
`PeekMessageW`（非阻塞）+ DDA 采集超时（8/32 ms）+ `!need_render` 时 8 ms sleep 下限，
静态推不出"永久停摆"路径。宿主现象证据（直读 exstyle / PostMessage 唤醒即恢复 /
sink 零事件）是决定性的。处理策略：按补丁实现兜底驱动源，并用 AC3 脚本在本地实测
复现——若本地复现则坐实修复打中根因；若不复现则记录为"宿主环境特有停摆路径"，
定时器兜底对其仍然有效（两者同构：都靠消息注入驱动循环）。
