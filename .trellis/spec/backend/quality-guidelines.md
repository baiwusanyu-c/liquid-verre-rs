# 质量约定

## 禁止模式

- **热路径（每帧）堆分配**：`Vec::new()` / `format!` / `env::var` / `to_string()` 进循环
  ⇒ 用 scratch 复用（`App::cmd_buf` / `snapshot` / `snapshot_key{,_next}`、
  `Compositor::prev_rects` / `cur_rects`）与启动期一次读取（`diag/env.rs`）。
  逐块投影要求 `Copy` 语义（`LiquidParams` / `DrawBlock` / `ResizeFeedback` 全是标量与定长数组）。
- **每帧重复 `sanitize`**：`LiquidParams::sanitize()` 有约 40 次 clamp，
  20 块 × 每帧 = 白烧 prep 预算 ⇒ 走 `core/style.rs::StyleCache`（键 = `id + style_epoch + scale 位模式`）。
- `ClearRenderTargetView` 充当「局部清屏」（它**忽略 scissor**）。
- 依赖 swapchain back buffer 跨帧持久（flip 模型不保）。
- GPU timestamp 进闭环判据（query 会打断驱动批处理，实测把帧率打到 15 fps，自我实现地降级）。
- **裸 `Instant - Duration`**（越界 panic，开机时间短时真炸过）⇒ 一律 `checked_sub` /
  `saturating_duration_since`；「很久以前」写 `Instant::now().checked_sub(Duration::from_secs(3600)).unwrap_or_else(Instant::now)`。
- **裸外部整数加减**（debug 下溢出即 panic）⇒ 先 clamp 或 widen 到 `i64`。
- **`f32::clamp` 夹可能是 NaN 的值**（NaN 会**原样返回 NaN**）⇒ 用 `api::config::clamp_finite`
  （NaN→lo，+inf→hi，-inf→lo）。NaN 进 GPU 会让 SDF 整块消失且不报错。
- **两处手写同一常量**：矩形/影响盒只在 `core/geometry.rs`；8 方向把手只在 `core/handle.rs`；
  有效圆角只在 `core/block.rs::effective_radius`；面板字段表只在 `playground/src/schema.rs`；
  画布窗口类名只在 `platform/window.rs::CANVAS_CLASS_NAME`（宽字符形式由它编译期展开）。
- **值类型加 `#[non_exhaustive]`**：`*Config` / `*Params` / `*Options` / `GlassStyle` /
  `WindowLevel` / `ZOrder` / `GlassMode` 一律**不加** —— 那会让上层没法写
  `WindowConfig { rect, ..Default::default() }`，而这恰恰是配置类 API 最重要的可用性。
  加字段靠 `Default` + serde `#[serde(default)]` 保持源码兼容。
  **只有** `ErrorKind` 与 `EngineEvent` 标 `#[non_exhaustive]`（那边加变体才是常态）。
- **面板 / 调试类代码进库**：浏览器页面、HTTP 服务器、演示布局全在 `playground/`。
  库里只允许保留 `tools/` 取证脚本依赖的确定性布局（`seed_diagnostic_blocks`）。
- **上层直接触碰 GPU 资源或 `Manager`**：一律走 `api::handle::Command` 队列 + `Snapshot` 读。
- 为指标好看而改取证脚本阈值 / 改日志前缀：阈值与前缀变更必须在 commit message 与
  `performance_report.md` 里写明理由（前缀是阀门接口，见 `logging-guidelines.md §2`）。
- **`cargo clippy` 豁免常驻化**：`#[allow(...)]` 必须写清为什么（例：
  `pyramid.rs` 的 `needless_range_loop` 是因为要同时索引三个并行数组）。
  `lib.rs` 有 `#![warn(missing_docs)]` + `#![warn(clippy::all)]`，公共项缺文档就是警告。

## 必须模式

- **视觉 / 管线改动附像素级或连拍证据**（oracle 选择见 `measurement-and-gates.md §3`）。
- **新增参数必须同时进 `playground/src/schema.rs`**：`liquid_fields()` /
  `transparent_fields()` / `resize_fields()` 各加一行，否则
  `every_param_is_exposed_in_the_panel` / `transparent_exposes_exactly_three_fields` /
  `resize_fields_cover_every_option` 单测直接 FAIL（需求 F1 的反向保证：
  不许出现「库支持但面板调不到」的死参数，也不许出现「面板有但库里没有」的死控件）。
  透明模式**必须**保持恰好 3 个字段（`corner_radius` / `opacity` / `color`，需求 F3.2）。
- **新增公共 API 必须走 `api/` + `lib.rs` 的 `pub use`**，并带文档注释（`missing_docs` 会报）。
- **新增 `GLASS_*` 开关**：一律 `GLASS_` 前缀、默认关闭或默认安全、
  登记进 README 的完整表；主循环要用的进 `diag/env.rs::EnvOverrides`，
  只属于某子系统构造期的可留在原地但要写进 `diag/env.rs` 的文档注释。
  能用 `EngineConfig` / `DiagnosticsConfig` 表达的，**优先结构化入口**（环境变量只是脚本后门）。
- **新增光标驱动的视觉效果**：先在 `core/geometry.rs` 给出它的有限支撑半径并纳入
  `frame_influence_px`，再写着色器（不变量 5，见 `rendering-pipeline.md §5`）。
- **新增 cbuffer 槽位**：只往后加（`c0..c17` 语义冻结），预留槽必须写零
  （dynamic cbuffer 的 `DISCARD` 映射会读到脏数据），并同步改
  `glass_ps.hlsl` 的注释、`glass_renderer.rs` 的 `CB_FLOATS` 与 `rendering-pipeline.md §3.4` 的表。
- 纯逻辑抽自由函数 + `#[cfg(test)]` 单测（矩形运算、sanitize、丢帧判据、clamp、EMA、
  把手几何、掩码打包、样式缓存、LUT 单调、块 clamp 已示范）。
- **中文注释写「为什么」**（含实测数字与前科），不写「是什么」。
  例：`LiquidParams::default()` 里 `blur_min: 20.0 * 0.12` 上方那段注释解释了
  「为什么不是 3.0、为什么必须写成表达式而不是字面量 2.4」——差 1 ULP 会多出约 2400 个
  ±1 LSB 差异像素。这类注释是取证结论的唯一载体，删掉等于让下一个人重踩。
- 提交前跑全套阀门；性能结论用轮转 A/B + 中位数，且只在机器健康窗口下给出。

## 测试要求

- **单测**：`cargo test --release --workspace`
  （当前：库 136 个 `#[test]` + 集成 11 + `playground` 5 个 schema 单测 + `lib.rs` / `api/engine.rs` 各 1 个
  `no_run` doctest）。新增纯逻辑**必须**带单测；`core/` 是主战场（零 Win32、零 `unsafe`，可测）。
  需要 GPU / 窗口的部分不做单测，改由 `tools/` 脚本覆盖（`runtime/commands.rs` 的注释里写明了这个分工）。
- **静态检查**：`cargo clippy --workspace --all-targets --release -- -D warnings` +
  `cargo fmt --all -- --check`（= `cargo xtask check` = `node scripts/check.mjs`）。
- **集成 / 视觉**：`tools/` 的 8 个回归脚本（清单见 `measurement-and-gates.md §5`）。
- **性能**：`python tools/gate_perf.py` 硬门槛不过不得提交
  （除非 commit message 说明是已知取舍并同步更新 BASELINE 指标表）。
- **改 CB 槽位 / 改默认参数 / 改 hover 相关视觉**：过一遍
  `.trellis/spec/guides/graphics-change-checklist.md` 的三问。