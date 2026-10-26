# PRD — P2 拖拽位置过滤 set_move_filter（可选，默认延后）

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
