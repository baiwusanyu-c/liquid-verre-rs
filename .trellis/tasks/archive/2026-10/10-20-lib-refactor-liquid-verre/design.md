# Design —— liquid-verre-rs

## 1. 目标形态（Cargo workspace）

```
liquid-verre-rs/                      # workspace root（原 tarui-glass）
├─ Cargo.toml                         # [workspace] members = crates/liquid-verre, playground, xtask
├─ crates/liquid-verre/               # 库 crate：package = liquid-verre-rs，lib name = liquid_verre
│  ├─ Cargo.toml
│  ├─ build.rs                        # 生成 OUT_DIR/build_info.rs（版本 / git sha / shader 摘要）
│  ├─ shaders/*.hlsl                  # 随库走（include_str!）
│  └─ src/…                           # 见 §2
├─ playground/                        # dev 控制面板（bin: liquid-verre-playground）
│  ├─ Cargo.toml                      # 依赖 liquid-verre-rs + tiny_http + serde_json
│  └─ src/{main.rs,panel.rs,web/index.html}
├─ xtask/                             # 构建 / 检查 / 打包 / 阀门（bin: xtask）
├─ scripts/{build.ps1,check.ps1,package.ps1,gate.ps1}
├─ tools/                             # 性能探针与像素取证脚本（EXE 路径改为 playground 产物）
├─ README.md / CLAUDE.md / AGENTS.md / BASELINE.md / performance_report.md / CHANGELOG.md
└─ .trellis/
```

为什么是 workspace 而不是「单 crate + example」：
- 控制面板带 `tiny_http` / `serde_json` 依赖，example 会把它们拖进库的依赖图（`cargo tree` 污染，
  上层应用被迫编译）；独立 crate 才能真正做到「playground 不是业务代码」。
- xtask 需要被 `cargo xtask` 调用，必须是 workspace 成员。
- workspace 共享一个 `target/`，`tools/*.py` 的 EXE 路径只需改一处常量。

## 2. 库内部分层（`crates/liquid-verre/src/`）

```
lib.rs            # 唯一公共出口：pub use api::*;  crate 级文档 + 快速上手示例
error.rs          # Error / ErrorKind / Result（#[non_exhaustive]，Display + std::error::Error + source）
logging.rs        # install_default_logger()：保留 [info]/[warn]/[boot]/[verify]/metrics 原始行格式
build_info.rs     # include!(concat!(env!("OUT_DIR"), "/build_info.rs"))
api/
  mod.rs
  config.rs       # EngineConfig / WindowConfig / GlassStyle / GlassMode / ResizeOptions / WindowLevel
  engine.rs       # Engine：new(config) -> Result<Self>; spawn()/pump()/shutdown(); 拥有 runtime
  handle.rs       # GlassWindow：Clone + Send + Sync 的句柄（命令队列 → 渲染线程）
  event.rs        # EngineEvent（块创建/销毁/缩放开始结束/层级变化/采集后端/错误），回调 + 通道两种订阅
core/             # 纯逻辑（可单测，零 Win32）
  params.rs       # Params（全参数）+ sanitize + scaled + cursor_influence_px + resize_influence_px
  style.rs        # GlassStyle = mode + LiquidParams/TransparentParams 的统一视图 ↔ Params 映射
  block.rs        # Block 几何 + clamp
  manager.rs      # 块集合 / z 序 / 命中测试 / handle 命中测试
  sdf.rs          # 折射 LUT + clamp_for_block + is_lod3
  quality.rs      # QualityCtrl（从 main.rs 抽出）
  geometry.rs     # 矩形运算（union/bbox/contains/hit/inflate），从 compositor.rs 抽出复用
runtime/
  mod.rs          # App（原 main.rs 的 App）+ 帧调度 + 事件驱动门控
  frame.rs        # FrameCtx / render_monitor / collect_monitor_blocks / sync_block_monitor
  commands.rs     # 上层命令（来自 GlassWindow 句柄）在渲染线程的消费点
platform/         # Win32 细节，全部 unsafe 集中在这里
  window.rs       # 窗口类 / CreateWindowExW / wndproc / SetWindowRgn / WS_EX_TRANSPARENT 切换
  level.rs        # WindowLevel → TOPMOST / BOTTOM / click-to-front
  display.rs      # monitors 枚举 + 刷新率 + 独占全屏检测 + 电源状态
gfx/              # 保持现状（device/shaders/pyramid/glass_renderer/compositor/dcomp/swapchain/gpu_timer）
capture/          # 保持现状（mod/dda/wgc）
input/            # cursor.rs / drag.rs / resize.rs（新增：8 handle 状态机）
diag/             # env.rs（GLASS_* 诊断开关集中解析）/ metrics.rs（Metrics + report_metrics）/ shots.rs
```

边界规则（沿用并强化 spec）：
- `api/` 是唯一 `pub` 面；`runtime/`、`platform/`、`gfx/`、`capture/`、`diag/` 全部 `pub(crate)`。
- `core/` 不得出现 `windows::` 依赖（保证可单测）。
- 所有 `unsafe` 集中在 `platform/` 与 `gfx/`、`capture/`；`api/` 与 `core/` 无 unsafe。
- 上层 → 库的调用一律走命令队列（`GlassWindow` 持 `Arc<Mutex<VecDeque<Command>>>` 或无锁 swap buffer），
  渲染线程在每帧开头一次性消费。**不允许**上层线程直接触碰 GPU 资源。

## 3. 线程模型

保持现状：**单渲染线程 + 消息泵**（库内 `Engine::spawn()` 起一个 OS 线程跑 `runtime`）。
上层通过 `GlassWindow` 句柄（`Send + Sync`）投递命令；事件通过回调（在渲染线程调用，文档写明）
或 `crossbeam`-free 的 `std::sync::mpsc::Receiver`（`Engine::events()`）向上抛。

为什么不用多线程：现有性能全部建立在「一次 Present / 一次 Commit / 脏区门控 / 事件驱动」上，
多线程只会引入锁与帧间不一致。库只要求上层「不要在回调里做重活」。

## 4. 配置 API（F1）

```rust
pub struct EngineConfig {
    pub default_style: GlassStyle,     // 所有窗口的默认外观
    pub window_level: WindowLevel,     // 画布窗口层级
    pub capture: CaptureConfig,        // 后端偏好 / 失效退避 / 空闲降频
    pub adaptive_quality: bool,        // 自适应降档（默认 true）
    pub fps_cap: Option<u32>,
    pub diagnostics: DiagnosticsConfig,// GLASS_* 对应的结构化开关
}

pub enum GlassStyle {
    Liquid(LiquidParams),        // 全部 ~40 个光学/交互参数
    Transparent(TransparentParams), // 只有 corner_radius / opacity / color
}

pub struct WindowConfig {
    pub rect: Rect,              // 逻辑坐标（主屏物理 px），与现有 Block 语义一致
    pub style: Option<GlassStyle>, // None = 用 engine 默认
    pub resize: ResizeOptions,
    pub level: Option<WindowLevel>,
    pub z_order: Option<i32>,
    pub label: Option<String>,
}
```

- `LiquidParams` = 现有 `Params` 的全字段（含面板没暴露的 `bevel_width` / `absorption` /
  `edge_always` / `spec_spread` / `rim_width` / `mouse_spread` / `mouse_color` / `proximity_glow` /
  `env_tint_mix` / `light_dir` / `light_intensity` / `shadow_margin` / `lag_seconds` /
  `progressive_curve` / `blur_min` / `super_n` / `shape` / `base_alpha` / `tint_color` / `profile`）。
- 所有 config 结构 `#[non_exhaustive]` + `Default` + `Serialize/Deserialize`（serde `rename_all="camelCase"`，
  与现有 `Params` 一致），支持上层用 JSON/TOML 直接喂。
- `sanitize()` 仍然是唯一入口：任何来自上层的值先 clamp 再进 GPU。
- **逐窗口 style**：`Params` 从「全局一份」改为「每块一份」（`Block.style`）。
  - LUT 只依赖 `ior` + `profile`：`GlassRenderer` 改为持有 **有界 LUT 缓存**（key = (ior 的位模式, profile)，
    容量 8，超出淘汰最旧），逐块 bind；相同 key 连续绘制时零额外上传。
  - Pass A 不依赖任何 `Params` 字段（`blur_max` 只进 c5 由 PS 选 LOD），因此逐块参数不影响金字塔共享。
  - `pyramid` 是否需要刷新改为「本屏是否存在 Liquid 块」（见 §6）。

## 5. 缩放交互（F2）

### 5.1 命中测试（CPU，`core/manager.rs` + `input/resize.rs`）

```rust
bitflags 风格（手写，不引依赖）：
pub struct ResizeEdges(u8);  // LEFT=1 RIGHT=2 TOP=4 BOTTOM=8
```
- `grip = resize.grip_px * block.scale`（默认 12 逻辑 px；对角命中盒 = 角部 grip×grip 的方框，
  与现有 `22.0 * scale` 的 SE grip 同量级，但改为可配）。
- 判定顺序：**先角后边**（角命中盒优先，避免角上同时命中两条边）。
- 命中盒只在 `resize.enabled` 且对应方向开启时才生效；关闭时退化为纯 `Move`（与今天一致）。
- 命中测试用「块外扩 grip」的矩形，因此需要把窗口的 `SetWindowRgn` 外扩量从 `shadow_margin`
  改为 `max(shadow_margin, grip + margin)`，否则边缘外侧拿不到鼠标。
  ⚠ 这会改变 `update_monitor_region` 的面积 → 必须同步核对 `verify_dirty` 的脏区结论。

### 5.2 拖拽几何（`input/resize.rs`）

`ResizeState { id, edges, origin:[x,y,w,h]（逻辑）, start_cursor, velocity, … }`
每帧 `apply(cursor, scale) -> [x,y,w,h]`（逻辑单位），锚定不动边：
- RIGHT: `w' = max(MIN, w + dx/s)`；LEFT: `x' = x + dx`, `w' = max(MIN, w - dx/s)`
  （命中 MIN 时 x 必须停住，否则左边会「滑」）；TOP/BOTTOM 同理。
- 角 = 两个方向的组合。
- 复用现有速度 EMA(0.75/0.25) 与 `lag_seconds` 外推，保证跟手性与 Move 一致。
- 跨屏：沿用 `sync_block_monitor`（中心过界换屏 + 按 DPI 比例换算），缩放期间同样适用。

### 5.3 视觉反馈（shader，`glass_ps.hlsl`）

新增 CB 槽（`CB_FLOATS` 18→22）：
- `c18 = (edgeL, edgeR, edgeT, edgeB)`：0/1，仅**被 hover 的块**非 0。
- `c19 = (cornerTL, cornerTR, cornerBL, cornerBR)`：0/1，同上。
- `c20 = (borderWidthPx, borderAlpha, 0, 0)`
- `c21 = (colorR, colorG, colorB, 0)`

mask 构造（`bw` = borderWidthPx，`aa` = c2.z 的 AA 宽度，`r` = c3.y 圆角，`hs` = 半尺寸，
`insideDistance` = SDF 内侧距离）：
```
// 直边：band × 「只在圆角切点之间」的窗
bandX = 1 - smoothstep(0, bw, hs.x - abs(local.x));          // 靠近左/右边
winY  = 1 - smoothstep(hs.y - r - aa, hs.y - r + aa, abs(local.y));  // 不进入圆角
edgeMaskLR = bandX * winY * (edgeL*step(local.x,0) + edgeR*step(0,local.x));
// Y 方向对称
// 对角：band × 「只在圆角象限」的窗 —— 沿弧拐弯
q = abs(local) - (hs - r);                                   // >0 即在圆角象限
cornerWin = smoothstep(-aa, aa, q.x) * smoothstep(-aa, aa, q.y);
cornerMask = (1 - smoothstep(0, bw, insideDistance)) * cornerWin * (选中角的 0/1);
fb = saturate(max(sum(edgeMask), sum(cornerMask)));
```
合成（保持 premultiplied 语义，源覆盖）：
```
outA = saturate(alpha + fb*c20.y*(1-alpha));
outC = col*glassAlpha + c21.rgb*fb*c20.y*(1-alpha);
```

**不变量对齐（关键）**：`fb` 只在 hover 块上非 0；而 Pass B 每帧整屏清 + 画**全部**块
（不变量 1），因此不存在「上一帧的 border 残留」。仍需保证：
- 光标影响盒 ⊇ 缩放反馈的支撑半径，新增单一来源
  `resize_influence_px(grip) = (grip*1.5 + 8) as i32`，
  并令 `cursor_influence_px` 的调用点改为 `max(cursor_influence_px(mouse_radius), resize_influence_px(grip))`。
- hover 变化必须触发一帧（现有 `hover_changed` 已覆盖）。
- 关闭 resize 反馈时 `c18/c19` 全 0 → `fb` 精确为 0 → **逐位等于旧渲染**（这是 A11 的保证）。

## 6. 双模式（F3）

- `GlassMode::Liquid`（默认）= 今天的完整管线。
- `GlassMode::Transparent` = 只做 SDF 覆盖 + 颜色 × 透明度：
  ```
  if (c22.x > 0.5) {
      float cov = glassCoverage;                 // 复用现有 SDF/AA 覆盖
      float a   = cov * c22.y;                   // c22.y = opacity
      return float4(c22.zw?…)                    // c23 = 颜色 rgb
  }
  ```
  新增 `c22 = (mode, opacity, cornerRadiusOverride, pad)`、`c23 = (r, g, b, pad)`。
  早退分支放在**所有纹理采样之前**，透明块的 PS 成本 ≈ 一次 SDF + 一次 AA。
- **Pass A 跳过**：`Compositor::render` 新增 `needs_backdrop: bool`（= 本屏存在 Liquid 块）。
  为 false 时不跑 Pass A、不动 `pyramid_valid`；0→1 跳变时调用 `invalidate()` 强制整块重算
  （走现有 `!covered` 分支，天然正确）。
- **采集降频**：`has_blocks` 判据扩展为 `has_blocks`（保持现有行为），
  另加 `needs_backdrop`；本屏全是透明块时按「空闲屏」4 Hz 保活采集，切回液态时
  复用 `just_activated` 强制阻塞取新帧的路径，不会闪旧桌面。
- 模式切换是**逐块**的（`Block.style`），运行期可切；切换即 `params_changed` → 触发一帧。

## 7. 层级（F4）

```rust
pub enum WindowLevel { AlwaysOnTop, Normal, ClickToFront, AlwaysOnBottom }
```
- `AlwaysOnTop`：今天的 `WS_EX_TOPMOST`（默认，保持视觉/性能不变）。
- `Normal`：去掉 TOPMOST，普通 popup，随其它窗口正常遮挡。
- `ClickToFront`：`Normal` + 点击命中块时 `SetWindowPos(HWND_TOP, NOACTIVATE|NOMOVE|NOSIZE)`。
- `AlwaysOnBottom`：`SetWindowPos(HWND_BOTTOM)` + 去掉 TOPMOST；仍在桌面之上（不能真的到桌面下面，
  否则玻璃不可见），文档写明语义 = 「在所有普通窗口之下」。
- 块之间 z 序：`Manager::bring_to_front / send_to_back / set_z / raise(id, delta)`，
  由 `GlassWindow` 句柄暴露。⚠ z 序变化必须触发 `geometry_changed`（否则不重画 → 叠放不更新）。
- 层级只影响**画布窗口**（每屏一个），因此是「整块玻璃层」的层级；
  单个玻璃块的层级 = 它在画布内的 z 序。这两层语义都要在 README 写清楚。

## 8. 错误与日志（E2）

```rust
pub enum ErrorKind {
    GraphicsInit, ShaderCompile, CaptureUnavailable, CaptureLost,
    MonitorEnumeration, WindowCreation, InvalidConfig, NotSupported, Internal,
}
pub struct Error { kind: ErrorKind, message: String, source: Option<Box<dyn StdError+Send+Sync>> }
```
- `Display` 输出 `[kind] message`；`std::error::Error::source()` 透传底层 HRESULT 文本。
- 现有 `Result<_, String>` 全部收敛为 `Result<_, Error>`；热路径不新增分配
  （错误构造只在失败分支）。
- 日志：`log::{trace,debug,info,warn,error}`；`logging::install_default_logger()` 提供
  **与今天逐字节相同的 stderr 行格式**（`[info] …` / `[warn] …` / `[boot] …` / `[verify] …` /
  `metrics …`），因为 `tools/*.py` 依赖这些前缀做解析。上层调用 `log::set_logger` 即可接管。
- 运行期可恢复错误（采集失效/退避）不返回 `Err`，走 `EngineEvent::Warning` + warn-once 日志
  （沿用现有 `capture_error_logged` 语义），避免上层被刷屏。

## 9. 构建 / 打包（E4）

- `crates/liquid-verre/build.rs`：
  - `cargo:rerun-if-changed=shaders`、`build.rs`、`../../Cargo.toml`
  - 生成 `OUT_DIR/build_info.rs`：`VERSION`、`GIT_SHA`、`GIT_DIRTY`、`BUILD_TIMESTAMP`、
    `TARGET_TRIPLE`、`SHADER_DIGEST`（每个 hlsl 的 SHA-256 前 12 位）
  - **不**在构建期编译 HLSL（保持运行期 D3DCompile，避免依赖 Windows SDK 的 fxc.exe；
    启动耗时实测 <50 ms，已在 `[boot]` 日志中打点）。
- `xtask`：
  - `build [--release] [--playground]`：workspace 构建
  - `check`：`fmt --check` + `clippy -D warnings` + `test --release`
  - `package [--out dist]`：构建后收集
    `target/release/liquid_verre.rlib`（+ `.dll` 若开 cdylib）、`liquid-verre-playground.exe`、
    `shaders/`、`README.md`、`LICENSE`、`build-info.json` → `dist/liquid-verre-rs-<version>-<triple>.zip`
  - `gate`：调用 `tools/gate_perf.py`
  - `clean`
- `scripts/*.ps1`：`cargo xtask <cmd>` 的薄封装（供不熟 cargo alias 的人用）。
- `.cargo/config.toml`：`[alias] xtask = "run --package xtask --"`。

## 10. 兼容与回滚

- 回滚点：tag `baseline/pre-refactor`、分支 `backup/pre-refactor-baseline`、重构起点 `4dca041`。
- 本次重构在**新分支** `codex/liquid-verre-rs-refactor` 上进行；每个阶段一次可编译提交点
  （但不自动 commit，等用户明确要求）。
- 分阶段落地顺序（每阶段结束都必须 build+test 绿）：
  1. workspace 骨架 + 文件搬迁（零行为变化）
  2. main.rs 拆分进 runtime/platform/diag（零行为变化）
  3. error/log 收敛（零行为变化，stderr 行格式不变）
  4. 配置 API + 逐块 style（默认路径逐位不变）
  5. 双模式 + Pass A 跳过
  6. 8 handle 缩放 + border 反馈（默认关闭时逐位不变）
  7. 层级
  8. playground 全参数面板
  9. 测试 / 工具 / 文档 / spec
  10. 文件夹重命名

## 11. 风险与对策

| 风险 | 对策 |
|---|---|
| `SetWindowRgn` 外扩到 grip 导致脏区/命中面积变化 | 外扩量走单一来源函数；`verify_dirty` 连跑 ≥3 次 |
| 逐块 Params 引入每块 `clone()`（今天已有）+ LUT 重绑 | LUT 有界缓存 + 按 z 序绘制，连续同 key 零上传；`params.clone()` 现状已存在，不新增 |
| 透明模式早退分支导致 PS 寄存器压力上升，拖慢液态块 | 分支放在函数最前 + `cb22.x` 是 per-draw 常量（无 warp 分歧）；用 gate_perf 的 own_us 核对 |
| CB 从 18→24 个 float4 影响 VS/PS | cbuffer 大小上限 4096 float4，无风险；VS 只用 c0/c16/c17 |
| 重命名文件夹后 IDE/工具路径失效 | 放在最后一步；`tools/*.py` 全部用 `ROOT` 相对路径 |
| 日志改走 `log` 后 stderr 格式漂移，tools 解析失败 | 默认 logger 严格保留原前缀；改完先跑 `perf_probe.py` 冒烟 |
