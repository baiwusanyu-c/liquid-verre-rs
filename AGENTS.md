<!-- TRELLIS:START -->
# Trellis Instructions

These instructions are for AI assistants working in this project.

This project is managed by Trellis. The working knowledge you need lives under `.trellis/`:

- `.trellis/workflow.md` — development phases, when to create tasks, skill routing
- `.trellis/spec/` — package- and layer-scoped coding guidelines (read before writing code in a given layer)
- `.trellis/workspace/` — per-developer journals and session traces
- `.trellis/tasks/` — active and archived tasks (PRDs, research, jsonl context)

If a Trellis command is available on your platform (e.g. `/trellis:finish-work`, `/trellis:continue`), prefer it over manual steps. Not every platform exposes every command.

If you're using Codex or another agent-capable tool, additional project-scoped helpers may live in:
- `.agents/skills/` — reusable Trellis skills
- `.codex/agents/` — optional custom subagents

Managed by Trellis. Edits outside this block are preserved; edits inside may be overwritten by a future `trellis update`.

<!-- TRELLIS:END -->

# 项目约定（liquid-verre-rs；Trellis 块之外，手工维护）

Cargo workspace 库项目：`crates/liquid-verre`（库，package `liquid-verre-rs`，**lib 名 `liquid_verre`**）
+ `playground`（dev 控制面板，bin `liquid-verre-playground`，**不是库的业务代码**）
+ `xtask`（构建/检查/打包/阀门 runner）+ `scripts/*.mjs`（Node，零 npm 依赖）。

## 基本约定

- 回答与文档用中文；代码标识符英文；中文注释写「为什么」并带实测数字与前科。
- 未经用户明确要求不得 git commit / push。
- 用户要求「临时提交一次代码」时只执行那一次；后续改动不自动 commit。
- PowerShell 写含中文文件前设 UTF-8（`[Console]::InputEncoding` / `$InputEncoding`）且 `-Encoding utf8`；
  **优先用 Python 写文件**（`io.open(path, "w", encoding="utf-8", newline="\n")`）。
  仓库现有 md 全是 **LF + 无 BOM**，别写成 CRLF。

## 提交前必须全绿（workspace 版）

```powershell
cargo test --release --workspace                                  # 库 136 + 集成 11 + playground 5 + doctest 2 = 154
cargo clippy --workspace --all-targets --release -- -D warnings
cargo fmt --all -- --check
node scripts/check.mjs                                            # 等价于 cargo xtask check
python tools/gate_perf.py                                         # 性能阀门（硬门槛 FAIL 退出码 1）
```

外加 BASELINE.md 的 **8 个回归脚本**：`verify_dirty`、`verify_mouse_isolation`、`verify_validity`、
`repro_tremble far`、`repro_tremble many`、`verify_adaptive`、`ab_latency`、`ab_memory`。
被测 EXE = `target/release/liquid-verre-playground.exe`（`VERRE_EXE` / `VERRE_OLD_EXE` 可覆盖）；
**库默认不自建窗口**，取证脚本一律显式设 `GLASS_DEFAULT_BLOCK=1`。

## 发版与 CI（本地助手 + GitHub Actions）

```powershell
node scripts/release.mjs --bump patch --dry-run   # 先看会做什么（不落盘、不动 git）
node scripts/release.mjs --bump patch             # 改版本号 + CHANGELOG 标题 + check + cargo package 干跑
node scripts/release.mjs --bump patch --ship      # 一键发版：上面全部 + commit + tag + push（触发发布流水线）
```

- `release.mjs` **默认不 commit / tag / push**（提交必须由人显式发起）。它把版本号
  （根 `Cargo.toml` 的 `[workspace.package] version`）、CHANGELOG 的
  `## [x.y.z-unreleased]` → `## [x.y.z] - 日期`、本地闸门（`node scripts/check.mjs`）与
  `cargo package -p liquid-verre-rs --list --allow-dirty` 干跑准备好，最后打印要人工敲的
  三条 git 命令。**工作区不干净会直接失败**（否则发出去的版本对不上 commit）。
  `--version x.y.z` 指定版本、`--tag` 覆盖 tag 名（必须 `v` + semver，否则不触发 `tags: ["v*"]`）、
  `--publish` 才在本地 `cargo publish`（排在 push 之前，失败时 tag 还没出去）。
- **`--ship` 是那句显式授权**：一条命令做完闸门 → 落盘 → `git add -A` + `commit -m "release: x.y.z"`
  （`--message` 可覆盖）→ 附注 tag → `git push origin HEAD:refs/heads/<branch> refs/tags/<tag>`，
  推完打印 Actions / Release 链接；`--ship --no-push` 只提交打 tag。两项预检排在闸门**之前**：
  git（detached HEAD / tag 重名 / 落后上游 / 无 origin）与 CHANGELOG（`--ship` 时必须有
  `## [x.y.z-unreleased]` 标题，否则 release notes 是空的 ⇒ 拒绝发）。
- `release.yml` 的 `crates-io` job **不能**用 `if: ${{ secrets.X != '' }}`：GitHub 明确规定 secret
  不能直接出现在 `if:` 条件里（写了就是静默跳过，配了 token 也不发布）。现实现是 job 只按 ref 过滤，
  token 有无在第一个 step 里用 job 级 `env` 判、没配就 `::notice::` 一行跳过。改这个 job 时别退回去。
- 推 `v*` tag 后 `.github/workflows/release.yml` 自动：windows 上 `check.mjs` + `package.mjs` →
  ubuntu 上从 CHANGELOG 抽 `## [x.y.z]` 段做 notes 建 GitHub Release（挂 dist zip + build-info.json）→
  配了 secret `CARGO_REGISTRY_TOKEN` 才 `cargo publish -p liquid-verre-rs --no-verify`，否则跳过。
- `.github/workflows/ci.yml`（push / PR）只跑 `fmt --check` + `clippy -D warnings` +
  `test --release --workspace` + `cargo package --list` 干跑；**像素取证与性能阀门不在 CI 跑**
  （依赖 DDA 桌面复制、真实 vsync 与可交互桌面会话，runner 上数字无意义），仍须在物理桌面上跑上面那套。
  CI 绿 ≠ 视觉/性能没回归。

## 分层边界（详见 .trellis/spec/backend/directory-structure.md）

- `api/` 是**唯一公共面**：`lib.rs` 只 `pub mod api` + `pub use`，其余模块一律 `pub(crate)`。
- `core/` 不得依赖 `windows::`（现状 0 引用、0 `unsafe`，因此全部可单测）；平台信息由 `runtime/` 传入。
- 所有 `unsafe` 集中在 `platform/`、`gfx/`、`capture/`，外加 `runtime/` 的平台点位
  （`RoInitialize`、DPI 感知、消息泵、`GWLP_USERDATA`、`DestroyWindow`）与 `input/cursor.rs`。
  **`api/` 与 `core/` 必须保持 0 个 `unsafe`。**
- 上层不得直接触碰 GPU 资源或 `Manager`，一律走 `Command` 队列（setter 异步、帧首一次性消费、
  延迟 ≤1 帧）+ `Snapshot` 读（最终一致）。
- 采集细节不得泄漏到 `gfx/`（经 `capture::Capture` 封装）；`gfx/` 原则上不读环境变量
  （例外清单登记在 `diag/env.rs` 的文档注释里，都只在构造期读一次）。
- 面板 / 调试类代码不得进库（全在 `playground/`）；库里只保留 `tools/` 依赖的确定性布局。
- **新增 `LiquidParams` 字段必须同时进 `playground/src/schema.rs::liquid_fields()`**，
  否则 `every_param_is_exposed_in_the_panel` 单测 FAIL（需求 F1 的反向保证）。
- 单一来源，禁止两处手写同一常量：矩形/影响盒 → `core/geometry.rs`；8 方向把手 → `core/handle.rs`；
  有效圆角 → `core/block.rs::effective_radius`；面板字段表 → `schema.rs`。
- `xtask/` 与 `scripts/` 只做编排、不存阈值；两条路径必须等价，改一边就改另一边。

## 不要破坏 BASELINE.md 的 9 条不变量

改 blur kernel / scissor 必须重算幂等外扩常量（`DOWN_GROW=[14,12,8,0]`、tent `32/24/16/8`、
`BLUR_REACH=96`、`HYSTERESIS=192`、`pad = 128 + min(vel*max_lag,128)`）并**连跑 `verify_dirty` ≥3 次**
（间歇残差只会在多次运行里露头）。

不变量 5 已扩容：光标驱动的视觉现在有**两项**（鼠标点光 `3R` 与缩放悬停反馈 `grip`），
影响盒 = `frame_influence_px(R, grip) + ceil(shadow_margin)` = `max(3R+32, 2*grip+16) + ceil(margin)`，
单一出口 `MonitorRt::influence_px()`；直边 / 对角掩码必须在**源头**互斥
（`to_shader_edges` 对角全 0、`to_shader_corners` 单边全 0），反馈只作用于被 hover 的那一块。

cbuffer：`c0..c17` 语义**冻结**（历史取证图的比对基准）；新增一律往后加
（`c18` = mode/opacity/handleGrabbed）；`CB_FLOATS = 19 * 4`。
缩放反馈 border 是独立叠加 pass（`shaders/border_vs|ps.hlsl` + `BorderCb`），不占用玻璃 cbuffer。

## 改这些地方之前必读（三个历史坑，都有实测数字）

1. **`tint_opacity: 0.01` 与 `blur_min: 20.0 * 0.12` 不是手误**，是旧二进制**稳态帧**真正渲染的值：
   旧面板 `SharedParams::apply()` 每帧写回、且用 `Arc<AtomicIsize>` 存值 ⇒ `0.018` 截断成 `0.01`、
   `blur_min` 被派生成 `blur_max*0.12`。旧二进制引导帧与稳态帧因此不同（实测 **873748 像素 /
   最大通道差 140**）。⚠ `blur_min` **必须写成表达式**：f32 下 `20.0*0.12 = 2.3999999`(`0x40199999`)，
   字面量 `2.4 = 2.4000001`(`0x4019999A`)，差 1 ULP ⇒ 多出约 **2400 个 ±1 LSB 差异像素**
   （interior 687 + ring 1679 + outside 4）。改这两个值 = 改视觉基线，须同步 BASELINE.md 与取证图。
2. **圆角半径统一由样式派生（`Block::effective_radius`）是有意的行为变更**：旧主循环
   `if target_id >= 0 { 改选中块 } else { 改全部块 }` + `sync_selection` 总把 `selected_id`
   兜底成最后一块 ⇒ 20 块里 **19 块**的 `radius` 永远停在硬编码 `28.0`。现在默认 32，
   因此跨版本比对 **many 布局圆角带必然不同**（`cmp_visual.py` 用圆角归因判据：
   `R_NEW=32`、`R_TOL=6`、`FAR_MAX_DIFF=2`）；single 布局仍必须四段全 0。
3. **`GLASS_CURSOR_AT` 现在全程钉死光标**（`App::pinned_cursor` / `App::read_cursor`）。
   过去只设初值、主循环立刻回到真实光标 ⇒「光标在玻璃内移动」用例**恒为 0 差异**（空转测试）。
   `cmp_visual.py` 另用 `SetCursorPos` 物理停放 + 看门狗，光标停在离玻璃 **> 3*mouse_radius** 处，
   让 `gate` 精确为 0。窗口过程与主循环都必须走 `read_cursor()`，否则命中测试与渲染用的光标会打架。

## 测量与取证

- 视觉 / 管线改动必须附像素级或连拍证据；oracle 选择见 `.trellis/spec/backend/measurement-and-gates.md`。
- 性能结论只在机器健康窗口下给出（`rel_max < 5 ms`、`acq_max < 25 ms`）；用轮转 A/B + ≥3 重复 + 中位数。
  **性能取证期间不要同时构建**（别的 agent 跑 `cargo build` / `cargo test` 就会把窗口打成劣化）。
- 跨版本视觉比**稳态帧**（`GLASS_INTERNAL_SHOT_AT_MS=2500`）+ 四段 deep/interior/ring/outside；
  时间维度用连拍（`GLASS_SHOT_EVERY_MS`）；`SIM_BACKDROP` 场景**禁止**做「增量==全量」像素对比。
- 视觉取证只能看 `GLASS_INTERNAL_SHOT` 的 composed 图（`WDA_EXCLUDEFROMCAPTURE` 使 OS 截屏看不到玻璃）。

## 日志与错误

- 库内一律用 `log` facade，**不自带后端**；`Engine::new()` 顺带装可选默认 stderr logger（上层已装则不覆盖）。
- **前缀即接口**：`RAW_TARGETS = boot / verify / metrics / adaptive / capture / probe`，其余走级别名。
  `[metrics]` → `perf_probe.py` / `gate_perf.py`；`[adaptive]` → `verify_adaptive.py`；
  `[verify]` → `verify_validity.py`；`[warn]` / `[info]` → `perf_probe.py` 摘要。**改前缀等于打断质量阀门。**
- 热路径禁止格式化日志；周期性告警一律 warn-once（状态位 + 翻转沿，或计数器 + 秒级节流）。
- 只有不可恢复的初始化失败才返回 `Err`；可恢复情况走 `EngineEvent::Warning` / 专用事件。
- `Instant` 相减一律 `checked_sub` / `saturating_duration_since`；禁止裸 `Instant - Duration`。
- 值类型（`*Config` / `*Params` / `*Options`）**不加 `#[non_exhaustive]`**；只有 `ErrorKind` / `EngineEvent` 加。
- NaN 用 `clamp_finite`（`f32::clamp` 遇 NaN 原样返回 NaN，NaN 进 GPU 会让 SDF 整块消失且不报错）。

## 参考

- D3D11/DXGI 静默陷阱清单：`.trellis/spec/backend/graphics-d3d11-gotchas.md`（hazard 丢绘制、
  Clear 忽略 scissor、flip 不持久、windows-0.39 `DuplicateOutput1` 绑定错误、HDR 复制面语义）。
  本项目锁 `windows = "0.58"`。
- 面向上层应用开发者的文档：`README.md`；变更清单：`CHANGELOG.md`；
  重构前基线：`BASELINE.md`；过程取证：`performance_report.md`。
- 发布 / 分发：`README.md` 的「发布与分发」一节（三种消费方式 + 发版流程 + 发布前需要提供什么）；
  流水线 `.github/workflows/{ci,release}.yml`；本地助手 `scripts/release.mjs`；
  crate 短 README `crates/liquid-verre/README.md`（`cargo package` 要求 readme 在 crate 目录内）。
- 回滚点：tag `baseline/pre-refactor`、分支 `backup/pre-refactor-baseline`（= `82fc311`）、
  重构起点 `4dca041`。