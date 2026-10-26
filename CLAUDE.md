# CLAUDE.md —— liquid-verre-rs（Windows 桌面玻璃窗口组件库）

Rust + 手写 D3D11 / DComp / WGC / DDA 的**玻璃窗口组件库**，实现《折射渲染方案》v1.1
（用户桌面 `折射渲染方案.md`）。上层应用创建一个玻璃窗口、配置外观，再把自己的 UI 叠在上面。

**Cargo workspace**：`crates/liquid-verre`（库，package `liquid-verre-rs`，**lib 名 `liquid_verre`**）
+ `playground`（dev 控制面板，bin `liquid-verre-playground`，**不是库的业务代码**）
+ `xtask`（构建/检查/打包/阀门 runner）。每屏一张全屏画布：采集 → 共享模糊金字塔
（Pass A，scissor；全透明屏整条跳过）→ 逐块着色（Pass B）→ 单 Present + DComp Commit；
事件驱动，静止即 0 帧。

## 常用命令

```powershell
cargo xtask build                # = cargo build --workspace --release
cargo xtask check                # = fmt --check + clippy -D warnings + test --release
cargo xtask gate                 # = 先 build，再 python tools/gate_perf.py
cargo xtask package              # 收集产物到 dist/ + build-info.json + zip
node scripts/check.mjs           # 与 cargo xtask check 等价（Node，零 npm 依赖）
node scripts/release.mjs --bump patch --dry-run   # 本地发版助手（只打印，不落盘；见 README「发布与分发」）
cargo run --release --bin liquid-verre-playground -- --port 8787   # dev 控制面板

cargo test --release --workspace                                  # 质量阀门 1（库 136 + 集成 11 + playground 5 + doctest 2 = 154）
cargo clippy --workspace --all-targets --release -- -D warnings
python tools/gate_perf.py                                         # 质量阀门 2（硬门槛 FAIL 退出码 1）
python tools/verify_dirty.py                                      # 视觉/一致性阀门之一（共 8 个，见 BASELINE.md）
```

被测 EXE 是 **`target/release/liquid-verre-playground.exe`**（`tools/*.py` 可用 `VERRE_EXE` /
`VERRE_OLD_EXE` 覆盖）。**库默认不自建窗口**，取证脚本一律显式设 `GLASS_DEFAULT_BLOCK=1`。

CI（`.github/workflows/ci.yml`）只跑 `fmt --check` + `clippy -D warnings` + `test --release --workspace`
+ `cargo package --list` 干跑；**像素取证与性能阀门一律不进 CI**（依赖 DDA 桌面复制、真实 vsync
与可交互桌面会话，runner 上测出来的数字无意义），必须在物理桌面上按上面这套命令跑。
发版：`node scripts/release.mjs --bump patch` 改版本号 + CHANGELOG 标题，然后由人
`git commit / git tag v<x.y.z> / git push --tags`（脚本刻意不代劳）；推 `v*` tag 后
`.github/workflows/release.yml` 自动建 GitHub Release，配了 `CARGO_REGISTRY_TOKEN` 才发 crates.io。

## 分层边界（详见 .trellis/spec/backend/directory-structure.md）

- `api/` 是**唯一公共面**（`lib.rs` 只 `pub mod api` + `pub use`）；新增公共项走这里，
  不许把内部模块直接 `pub` 出去。
- `core/` 是纯逻辑：现状 **0 个 `windows::` 引用、0 个 `unsafe`**，因此全部可单测。
  需要平台信息（DPI / 光标 / 显示器几何）时由 `runtime/` 取值后**作为参数传入**。
- `unsafe` 只允许在 Win32 / GPU / COM 边界：`platform/`、`gfx/`、`capture/`，
  以及 `runtime/` 的平台点位（`RoInitialize`、DPI 感知、消息泵、`GWLP_USERDATA`、`DestroyWindow`）
  与 `input/cursor.rs`（`GetCursorPos`）。**`api/` 与 `core/` 必须保持 0 个。**
- 上层不得直接触碰 GPU 资源或 `Manager`：一律走 `Command` 队列（setter 异步，帧首一次性消费，
  延迟 ≤1 帧）+ `Snapshot` 读（最终一致）。
- 采集细节不得泄漏到 `gfx/`（经 `capture::Capture` 封装）。
- **面板 / 调试类代码不得进库**（浏览器页面、HTTP 服务器、演示布局全在 `playground/`）；
  库里只保留 `tools/` 依赖的确定性布局（`seed_diagnostic_blocks`）。
- **新增 `LiquidParams` 字段必须同时进 `playground/src/schema.rs::liquid_fields()`**，
  否则 `every_param_is_exposed_in_the_panel` 单测直接 FAIL。
- 单一来源，禁止两处手写同一常量：矩形/影响盒 → `core/geometry.rs`；8 方向把手 →
  `core/handle.rs`；有效圆角 → `core/block.rs::effective_radius`；面板字段表 → `schema.rs`。

## 不可破坏的不变量（详见 BASELINE.md / .trellis/spec/backend/rendering-pipeline.md §5）

1. 每帧自包含：整屏清 + Pass B 画全部块（flip back buffer 不持久）；Pass A 按脏区 scissor。
   `ClearRenderTargetView` **忽略 scissor**。
2. 金字塔有效性只声称真正重算过的区域；`!covered` 先于 `backdrop_changed`。
   全透明屏既不重算也不声称。
3. 模糊链幂等外扩常量：`DOWN_GROW = [14,12,8,0]`、tent 链 `g = 32 - 8*pass`（32/24/16/8）、
   `BLUR_REACH = 96`、`HYSTERESIS = 192`、`pad = 128 + min(vel*max_lag, 128)`。
4. 环境色 EMA 只在捕获内容真变化时推进（否则拖拽时全体块 ±1~2 LSB 色调跳变）。
5. **一切光标驱动的视觉都是有限支撑窗，影响盒必须严格覆盖**：鼠标点光 `3R`
   （`cursor_influence_px = 3R+32`）**与缩放悬停反馈 `grip`**（`resize_influence_px = 2*grip+16`）
   取大者，再加 `ceil(shadow_margin)`，出口是 `MonitorRt::influence_px()`（单一来源）。
   配套：直边 / 对角掩码在**源头互斥**（`to_shader_edges` 对角全 0、`to_shader_corners` 单边全 0）；
   反馈只作用于被 hover 的那一块（其余块 `ResizeFeedback::NONE` → border pass 直接跳过该块）。
6. 脏区三来源（桌面脏区 / 位移 / 光标盒）始终并集，不得短路。
7. HDR 输出自动切 WGC（DDA 复制面是 HDR 参考值，会过曝）。
8. 自适应控制器判据只看「我们自己的工作」（prep+A+B+Commit），排除采集等待与 Present 阻塞。
9. 元数据与上层输入先 sanitize 再运算（debug 溢出即 panic 的前科）；NaN 用 `clamp_finite`
   （`f32::clamp` 遇 NaN **原样返回 NaN**，NaN 进 GPU 会让 SDF 整块消失且不报错）。

cbuffer：`c0..c17` 语义**冻结**（历史取证图的比对基准），新增一律往后加
（`c18` = mode/opacity/handleGrabbed）；`CB_FLOATS = 19 * 4`。
   缩放反馈 border 是**独立叠加 pass**（`shaders/border_vs|ps.hlsl` + `BorderCb` b0..b6，
   `GlassRenderer::draw_borders` 在 Pass B 之后画），不占用玻璃 cbuffer。

## 改这些地方之前必读（四个历史坑，都有实测数字）

1. **默认参数 `tint_opacity: 0.01` 与 `blur_min: 20.0 * 0.12` 不是手误。**
   它们是重构前那个二进制**稳态运行时真正渲染出来的值**：旧面板 `SharedParams::apply()`
   每帧把参数写回，而它用 `Arc<AtomicIsize>` 存值 ⇒ `0.018` 被截断成 `0.01`、
   `blur_min` 被派生成 `blur_max*0.12`。旧二进制的引导帧与稳态帧因此不同
   （实测相差 **873748 像素、最大通道差 140**）。
   ⚠ `blur_min` **必须写成表达式**而不是字面量 `2.4`：f32 下 `20.0*0.12 = 2.3999999`
   （`0x40199999`），字面量 `2.4 = 2.4000001`（`0x4019999A`），差 1 ULP，
   用字面量会多出约 **2400 个 ±1 LSB 差异像素**（interior 687 + ring 1679 + outside 4）。
   改这两个值 = 改视觉基线，必须同时更新 BASELINE.md 与取证图。
2. **圆角半径统一由样式派生（`Block::effective_radius`）是有意的行为变更。**
   旧主循环是 `if target_id >= 0 { 改选中块 } else { 改全部块 }`，而 `sync_selection`
   总会把 `selected_id` 兜底成最后一块，于是 20 块布局里有 **19 块**的 `radius` 永远停在
   `Block::new` 硬编码的 `28.0`。现在统一按样式派生（默认 32），
   因此跨版本比对 **many 布局时圆角带必然不同**（`cmp_visual.py` 用圆角归因判据处理，
   `R_NEW=32`、`R_TOL=6`、`FAR_MAX_DIFF=2`）；single 布局仍必须四段全 0。
3. **`GLASS_CURSOR_AT` 现在全程钉死光标**（`App::pinned_cursor` / `App::read_cursor`）。
   过去它只设初值、主循环立刻回到真实光标，导致「光标在玻璃内移动」那个取证用例
   **恒为 0 差异**（空转测试，什么也没证明）。`cmp_visual.py` 另外用 `SetCursorPos`
   物理停放 + 看门狗，并把光标停在离玻璃 **> 3*mouse_radius** 的地方，
   让鼠标点光的有限支撑窗 `gate` 精确为 0。窗口过程与主循环都必须走 `read_cursor()`，
   否则命中测试用真实光标、渲染用钉死光标会打架。

4. **库侧焦点代调是死路，别写回来（2026-10-26 实锤）**：跨线程 `SetFocus` 恒返 NULL
   （`ret=0x0`），`AttachThreadInput` 桥接会挂死渲染线程（实测：按压后取证心跳停摆、
   进程活着玻璃停绘）。终态 = `set_focus_hwnd` 登记 + 按压消费时发 `EngineEvent::FocusHint`，
   宿主在自己线程 SetFocus；守卫拒绝画布自身 HWND。库内禁止再出现 SetFocus/attach FFI。

## 日志与错误

- 库内一律用 `log` facade，**不自带后端**；`Engine::new()` 顺带装可选的默认 stderr logger
  （上层已装则不覆盖）。想接管就在 `Engine::new()` **之前** `log::set_logger(...)`。
- **前缀即接口**：`RAW_TARGETS = boot / verify / metrics / adaptive / capture / probe`，
  默认 target 走级别名 `[info] / [warn] / [error]`。
  `[metrics]` → `perf_probe.py` / `gate_perf.py`；`[adaptive]` → `verify_adaptive.py`；
  `[verify]` → `verify_validity.py`；`[warn]` / `[info]` → `perf_probe.py` 的摘要。
  **改前缀或字段名等于打断质量阀门。**
- 热路径禁止格式化日志；周期性告警一律 warn-once（状态位 + 翻转沿，或计数器 + 秒级节流）。
- 只有**不可恢复的初始化失败**才返回 `Err`；采集失效 / 独占全屏 / DPI 感知被宿主占用 /
  句柄指向已销毁窗口这类可恢复情况走 `EngineEvent::Warning` 或专用事件，
  **绝不**做成 `Err`（锁屏期间每 500 ms 一次，会刷屏）。
- 时间运算：`Instant` 相减一律 `checked_sub` / `saturating_duration_since`，**禁止裸 `Instant - Duration`**。
- 值类型（`*Config` / `*Params` / `*Options`）**不加 `#[non_exhaustive]`**
  （会让上层没法写 `WindowConfig { rect, ..Default::default() }`）；
  只有 `ErrorKind` 与 `EngineEvent` 加。

## 测量规则（本机结论的唯一合法来源）

- 轮转交错 A/B + ≥3 重复 + 中位数/p95；单次运行的内存/fps 数字不可信。
- 机器健康判据：`rel_max < 5 ms`、`acq_max < 25 ms`；劣化窗口的数据作废。
- **性能取证期间不要同时构建**（另一个 agent 跑 `cargo build` / `cargo test` 就会把窗口打成劣化）。
- oracle 选择表见 `.trellis/spec/backend/measurement-and-gates.md`
  （`SIM_BACKDROP` 场景禁止做「增量==全量」像素对比；时间维度用连拍 `GLASS_SHOT_EVERY_MS`；
  跨版本视觉用**稳态帧** `GLASS_INTERNAL_SHOT_AT_MS=2500` + 四段 deep/interior/ring/outside）。
- 视觉取证只能看 `GLASS_INTERNAL_SHOT` 的 composed 图（`WDA_EXCLUDEFROMCAPTURE` 使 OS 截屏看不到玻璃）。

## 协作约定

- 回答与文档用中文；代码标识符英文；中文注释写「为什么」并带实测数字与前科。
- **未经用户明确要求不得 git commit / push。**
- **PowerShell 中文编码强制规则**：写含中文（或任何非 ASCII）的文件前必须先
  `[Console]::InputEncoding = [System.Text.Encoding]::UTF8` 与
  `$InputEncoding = [System.Text.Encoding]::UTF8`，写文件时显式 `-Encoding utf8`；
  **优先用 Python 写文件**（Python 3 默认 UTF-8，不受 PowerShell 编码影响），
  例如 `io.open(path, "w", encoding="utf-8", newline="\n")`。仓库现有 md 全是 **LF + 无 BOM**。
- **Codex worktree 依赖规则（panda-claw-web）**：禁止在
  `C:\Users\chenzhihui\.codex\worktrees\**\panda-claw-web` 内执行 `pnpm install`
  （每个任务复制一份完整 node_modules，浪费磁盘）。worktree 缺 node_modules 时创建 Junction
  复用主项目（dev-v2.1）依赖：
  `New-Item -ItemType Junction -Path <worktree>\node_modules -Target D:\chapanda-project\panda-claw-web\node_modules`。
  若 worktree 已有实体 node_modules（非链接），先确认无任务在跑，再删除并改建上述 Junction。
- 新开关一律 `GLASS_*` 前缀、默认关闭或默认安全、README 登记；能用 `EngineConfig` /
  `DiagnosticsConfig` 表达的优先结构化入口。
- `xtask/` 与 `scripts/` 只做编排、不存阈值；两条路径必须等价，改一边就改另一边。
- 回滚点：tag `baseline/pre-refactor`、分支 `backup/pre-refactor-baseline`（= `82fc311`）、
  重构起点 `4dca041`。
- D3D11/DXGI 静默陷阱清单：`.trellis/spec/backend/graphics-d3d11-gotchas.md`
  （hazard 丢绘制、Clear 忽略 scissor、flip 不持久、windows-0.39 `DuplicateOutput1` 绑定错误、
  HDR 复制面语义）。本项目锁 `windows = "0.58"`。