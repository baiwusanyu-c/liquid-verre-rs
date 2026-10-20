# 质量约定

## 禁止模式

- 热路径（每帧）堆分配：`Vec::new()`/`format!`/`env::var` 进循环 ⇒ 用 scratch 复用与启动期一次读取。
- `ClearRenderTargetView` 充当"局部清屏"（忽略 scissor）。
- 依赖 swapchain back buffer 跨帧持久。
- GPU timestamp 进闭环判据。
- 裸 `Instant - Duration`、裸外部整数加减。
- 两处手写同一常量（影响盒半径已有 `cursor_influence_px` 单一来源；gate 支撑半径 3R 在 shader 注释里引用它）。
- 为指标好看而改取证脚本阈值：阈值变更必须在 commit message 与 performance_report 里写明理由。

## 必须模式

- 视觉/管线改动附**像素级证据**（oracle 选择见 measurement-and-gates §3）。
- 新开关一律 `GLASS_*` 前缀、默认关闭或默认安全、README 登记。
- 纯逻辑抽自由函数 + `#[cfg(test)]` 单测（矩形运算、sanitize、判据、clamp、EMA 已示范）。
- 中文注释写"为什么"（含实测数字与前科），不写"是什么"。
- 提交前跑 BASELINE.md 的全部门阀；性能结论用轮转 A/B。

## 测试要求

- 单测：`cargo test --release`（当前 25 个；新增纯逻辑必须带单测）。
- 集成/视觉：8 个 tools 脚本（见 measurement-and-gates §5）。
- 性能：`gate_perf.py` 硬门槛不过不得提交（除非 commit message 说明是已知取舍并更新 BASELINE 指标表）。
