# PRD — P2 拖拽位置过滤 set_move_filter,,> **裁决（2026-10-26，用户确认方向 + 主会话论证）：不做，从 0.5.0 范围移除并归档。**,> 理由：① P0-2 的 `Consumed` 已提供正解——需要吸附的块由宿主在 sink 回调里接管拖拽，,> 自己 `set_rect` 驱动吸附位置（异步入队、≤1 帧，与库会话同等延迟）；不需要吸附的块,> 继续用库的 movable 拖拽，两种块 per-block 共存。② 库开会话 + 宿主外部 set_rect 是,> 互相抢状态的反模式（会话每帧从光标锚点回算，外部修正被覆盖 → 抖动）。③ 注入点式,> API 违背已拍板边界「库只做玻璃，窗口本体归宿主」，热路径 Fn 契约成本配不上省下的,> 宿主 ~30 行。宿主对接写法见 README「宿主集成样例」。

> 前置：父任务 design.md §4。独立小任务，不与 P0/P1 互相阻塞；默认排在 0.5.0 集成评审之后，
> 由用户决定是否进本轮。

## 需求

- FR1 `GlassWindow::set_move_filter(&self, Option<Arc<dyn Fn([f32;4]) -> [f32;4] + Send + Sync>>)`：
  移动/缩放会话内每帧把候选块矩形（逻辑像素 `[x0,y0,x1,y1]`）交给 filter，返回值经
  `clamp_finite` sanitize 后替换候选矩形，再走内置屏幕 clamp。None = 现行为、零开销。
- FR2 契约同 sink（渲染线程会话内同步、微秒级、不阻塞不重入；写进 rustdoc）。
- FR3 filter 抛 panic/返回 NaN ⇒ 该帧候选作废（用上帧矩形兜底）+ warn-once。

## 验收标准

- AC1 单测：透传/替换/NaN 兜底/未设置零调用（core 0-unsafe 侧）。
- AC2 demo + 取证：playground 两块对齐吸附演示，连拍（`GLASS_SHOT_EVERY_MS`）证明
  「吸附跟手、无松手跳变」；拖拽帧不破 `gate_perf`/性能阀门。
- AC3 第一门禁全套（父 §4）后合入；README/CHANGELOG。
