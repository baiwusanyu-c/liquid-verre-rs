# Changelog

本文件记录本项目的所有显著变更。

格式遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [SemVer](https://semver.org/lang/zh-CN/)。

重构前的历史（单 bin crate `liquid-glass` 时期的第一~十四轮性能与视觉取证）见
`performance_report.md` 与 `BASELINE.md`，本文件从库化重构那一版开始记。

## [0.4.1] - 2026-10-21

### Added
- **「是否可拖拽移动」参数（两种玻璃模式共用）**：`WindowConfig.movable`（默认 `true`，
  serde `camelCase` 下就是 `movable`）、`GlassWindow::set_movable(bool)`、
  `Command::SetMovable { id, movable }`，并沿 `runtime/app.rs::SpawnSpec` → `core/block.rs::Block`
  → `core/draw.rs::DrawBlock` → `api/handle.rs::WindowSnapshot` 全链贯通（快照可读回）。
  语义：`false` 时按住块**内部**不开始移动会话（`platform/window.rs::on_button_down` 早退，
  不 `SetCapture`、不发 `MoveStarted`），但**缩放把手仍然有效**
  （`Manager::hit_resize` 只看 `interactive && visible && resize.enabled`，不看 `movable`），
  点击抬层也仍然生效（同层 `bring_to_front` 与 `WindowLevel::ClickToFront`）。
  正在移动中的块被设为 `false` 会**立刻结束**移动会话（`runtime/commands.rs` 显式丢弃
  `DragSession::Move` 并清 `press`），否则它会继续跟着光标跑；缩放会话不受影响。
  `movable` 不参与渲染，因此 `apply_one` 对 `SetMovable` 返回 `false`（不脏化绘制账本）、
  只置 `snapshot_dirty`。单测：`movable_defaults_to_true_and_survives_json`、
  `movable_toggle_enqueues_its_own_command`、`new_blocks_are_movable_and_interactive_by_default`。
- **playground 面板**：窗口区新增「可拖拽移动」按钮
  （`POST /api/window {"action":"movable","id":..,"value":..}` → `GlassWindow::set_movable`），
  文案随状态在「可拖拽移动 / 禁止拖拽移动」之间切换，`GET /api/state` 的每个窗口回显 `movable`。
- **GitHub Actions 流水线**：
  - `.github/workflows/ci.yml`（每次 push / PR，`windows-latest`）：`cargo fmt --all -- --check` +
    `cargo clippy --workspace --all-targets --release -- -D warnings` +
    `cargo test --release --workspace` + `cargo package -p liquid-verre-rs --list` 干跑。
    **刻意不跑**像素取证（`cmp_visual` / `verify_*` / `repro_tremble`）与性能阀门（`gate_perf`）：
    它们依赖 DDA 桌面复制、真实 vsync 节拍与可交互的桌面会话，CI 的无交互会话里测出来的
    数字没有意义（口径见 `.trellis/spec/backend/measurement-and-gates.md §1`）。
  - `.github/workflows/release.yml`（`v*` tag 或 `workflow_dispatch`）：`windows-latest` 上
    `node scripts/check.mjs` + `node scripts/package.mjs` → 上传 `dist/` artifact →
    `ubuntu-latest` 上从 `CHANGELOG.md` 抽 `## [x.y.z]` 段当 notes 建 GitHub Release
    （挂 `dist/*.zip` + `dist/*/build-info.json`，版本号含 `-` 自动标 prerelease）→
    若配置了 secret `CARGO_REGISTRY_TOKEN` 则 `cargo publish -p liquid-verre-rs --no-verify`，
    否则整个 job 跳过（git 依赖也是完全可用的分发方式）。
- **`scripts/release.mjs`（本地发版助手）**：工作区必须干净 → 算目标版本
  （`--bump patch|minor|major` 或 `--version x.y.z`，tag 名默认 `v<version>`、`--tag` 可覆盖但必须
  是 `v` + semver，否则不匹配 `release.yml` 的 `tags: ["v*"]`）→ **git 预检**（detached HEAD /
  tag 本地或远端重名 / 落后上游 / 没有 origin）→ **CHANGELOG 预检** → `node scripts/check.mjs` →
  `cargo package -p liquid-verre-rs --list --allow-dirty` 干跑 → 改根 `Cargo.toml` 的
  `[workspace.package] version` + 把 CHANGELOG 的 `## [x.y.z-unreleased]` 标题替换成
  `## [x.y.z] - YYYY-MM-DD` → 打印接下来要人工执行的 `git commit / git tag / git push`。
  git 与 CHANGELOG 两项预检都排在闸门**之前**：tag 重名这类错误不该等 5 分钟 clippy 才发现。
  **默认不 commit / tag / push**（项目约定：提交必须由人显式发起）；`--dry-run` 只打印不落盘，
  `--publish` 才在本地直接 `cargo publish`（排在 push 之前，失败时 tag 还没出去，会打印回滚命令）。
- **`scripts/release.mjs --ship`（一键发版）**：同一条命令做完「闸门 → 改版本号 + CHANGELOG →
  `git add -A` + `commit -m "release: x.y.z"`（`--message` 可覆盖）→ 附注 tag `vx.y.z` →
  一条 `git push origin HEAD:refs/heads/<branch> refs/tags/<tag>`」，tag 落地即触发
  `release.yml`，最后打印 Actions / Release 链接。`--ship --no-push` 只提交打 tag、把 push 留给人；
  `--ship` 时若 CHANGELOG 没有 `## [x.y.z-unreleased]` 标题会**拒绝发布**（否则 release notes 是空的）。
  沙盒验证过全链路：真实 git 仓库 + 本地 bare 远端，push 后 `git ls-remote --tags` 能看到 tag；
  重名 tag / detached HEAD / 脏工作区 / 非法 `--bump` / `--no-push` 不带 `--ship` 五条错误路径都有明确报错。
- **`crates/liquid-verre/README.md`**：crate 自带短 README —— `cargo package` 要求 `readme`
  指向 **crate 目录内**的文件（workspace 根的 README 不会被打进 `.crate`）；
  `crates/liquid-verre/Cargo.toml` 的 `readme` 已指向它，`repository` 已填
  `https://github.com/baiwusanyu-c/tarui-glass`（缺失时 `cargo package` 只告警不失败，
  但 crates.io 页面会少一个「源码」链接；填完后 `cargo package --list` 已无告警）。

### Fixed
- **`release.yml` 的 crates.io job 永远不会跑**（发布链路）：job 级 `if:` 里写了
  `${{ secrets.CARGO_REGISTRY_TOKEN != '' }}`，而 GitHub 明确规定 **secret 不能直接用在 `if:` 条件里**
  （官方安全指南原话："Secrets cannot be directly referenced in if: conditionals"）⇒ 即使配好 token
  也会静默跳过，Actions 页面上看不出原因。改为 job 只按 ref 过滤
  （`startsWith(github.ref, 'refs/tags/v') || github.event.inputs.tag != ''`），token 有无放到第一个
  step 里用 job 级 `env` 判；没配就留一条 `::notice::` 并跳过后续 step（GitHub Release 与 git 依赖不受影响）。
- **API 改 z 序不重画**（P0）：`bring_to_front` / `send_to_back` / `set_z_order` 只改
  `Manager` 不脏化合成器，静止桌面上端到端无效。新增 `Compositor::invalidate_present()`
  （只作废 present 账本、不动金字塔账本），`SetZ` 变化时调用。
- **引擎级默认样式切换不脏化**（P1）：`SetDefaultStyle` 补 invalidate；并把
  「有效样式」收敛为单一来源 `Block::effective_style(default)` —— 此前采集门控读
  陈旧的 `b.style`，`set_default_style(transparent)` 后 DDA 采集不会降频，
  正好抵消透明模式省采集的收益。
- **border 原点与玻璃 pass 不同源**（P1）：`draw_borders` 曾把矩形 clamp 到 ≥0，
  块半出屏时白框整体错位。改为统一走 `DrawBlock::draw_rect()`（未 clamp），clamp 只给 scissor。
- **`interactive=false` 不吃点击**（P1）：穿透判定此前按全部可见块算，不可交互块也会
  摘掉 `WS_EX_TRANSPARENT` 跨进程吃掉点击。`DrawBlock` 增加 `interactive` 字段，
  穿透判定排除它（窗口区域仍包含，因为它要画出来）。
- **`AlwaysOnBottom → Normal/ClickToFront` 不恢复 Z 序**（P2）：`level::apply` 的
  「样式没变就跳过」早退吃掉这次转换；早退删除（调用方已做 level 幂等短路）。
- **`QualityCtrl::disable()` 误清 `fps_cap`**（P1）：`GLASS_ADAPTIVE=0`（每个 verify_* 都设）
  会让帧率上限静默失效；`disable()` 现在只清自适应那一份间隔。
- **`EngineConfig::adaptive_quality` 无读取点**（P2）：接线为「与 `diagnostics.no_adaptive`
  / `GLASS_ADAPTIVE=0` 任一关闭即禁用」。
- **`GlassWindow` 读-改-写 setter 同帧互相覆盖**（P2）：`set_corner_radius` / `set_opacity` /
  `set_color` / `set_resizable` 改为细粒度命令（`Command::PatchStyle` / `SetResizable`），
  在渲染线程的权威状态上叠加。⚠ 行为变更：补丁过字段的窗口会被固化（`inherits_style=false`），
  已写进文档。
- **引擎退出后命令队列无限增长 / `start()` 可双开渲染线程**（P2）：`Shared` 增加
  `stopped` / `started` 位；退出后 setter no-op，重复 start 返回 `Err(Shutdown)`。
- **全透明屏谎报 `background_valid`**（P2，不变量 2）：退化脏区早退分支补 `needs_backdrop` 保护。
- **采集运行期重建丢后端偏好**（P1）：改走 `for_monitor_pref(.., backend_pref)`；
  零调用点的 `Capture::for_monitor` 删除。
- **`GLASS_SIM_BACKDROP` 脏区量纲错**（P2）：此前把虚拟屏绝对坐标当画布局部坐标塞给每块屏，
  主屏原点非 (0,0) 的排布下确定性负载静默失效；现按屏换算。
- **透明块以 `mouse_radius=20` 参与影响盒**（P2）：透明模式输出与光标无关，却会让光标
  白白驱动整帧重画；改按 `mouse_intensity > 0` 门控（`max_grip` 不门控，border 反馈对透明块同样生效）。

### Changed
- `Manager::hit_resize` 改零分配单遍扫描（热路径零堆分配）；方向开关判据改走
  `ResizeOptions::edges_active()/corners_active()` 单一来源。
- `EngineEvent::CaptureLost` 增加 `kind: ErrorKind` 字段（上层按枚举分支，不匹配字符串）。
- 性能阀门新增第四个场景 `transparent-drag`（需求 F3.1 / 验收 A9），硬门槛与液态单块同一套。
- `cmp_visual.py` 的 oracle 修正（两处都是取证框架缺陷，不是产品行为）：
  ① 一律设 `GLASS_ENV_EMA=0` 冻掉环境色 EMA 的墙钟 dt 依赖（合成源只替换纹理，
  `changed` 仍来自真实桌面，桌面一活跃 EMA 推进时刻就不同 ⇒ 全图 ±1~2 LSB 漂移）；
  ② many 布局的 now-vs-now 判据从「硬 0」改为「每段最大通道差 ≤ 2 LSB」——
  frost/16 层按脏区部分刷新，跨运行刷新序列在桌面繁忙时差 ±1 LSB，属重构前既有特性，
  同一次运行内「增量 == 全量」由 `verify_dirty.py` 严格证明；single 布局仍硬 0。
- **缩放反馈 border 默认加粗**：`ResizeFeedbackOptions::default()` 的 `border_width`
  `3.0 → 5.0`（逻辑像素；投影时 `DrawBlock::feedback.width = border_width * scale`，
  200% DPI 下 = 10 物理像素）、`alpha` `0.95 → 1.0`。
  原因：`3 px / 0.95` 在 100% DPI 屏上实测「看得见但不醒目」，用户验收时反馈触发不够明显；
  5 逻辑 px 在 200% DPI 是 10 物理 px 的纯白实线、在 100% DPI 也有 5 px，两种屏都一眼可辨。
  同轮还修了「看着细」的根因：band 原先是 `1 - smoothstep(0, bw, inside)`，从第 1 像素就开始
  渐隐，像素剖面实测 `bw=5` 时 t=2 处 alpha 已掉到 0.49、t=4 归零，人眼只读得到约 2 px 白线；
  现改为**实芯 + 外沿 AA**：`aaBand = min(aa, bw * 0.5)`、`band = 1 - smoothstep(bw - aaBand, bw, inside)`，
  `t < bw - aaBand` 全不透明、最后 `aaBand` 像素做抗锯齿（实测剖面 t=1..3 纯白 255、t=4 一个 AA 过渡像素；
  面板下发 `borderWidth=20` 得到精确 19 px 实芯 + 1 px AA，厚度完全受控）。直边 `bandX/bandY` 同款公式。
  band 仍是 SDF **内侧**距离（`inside = max(-sd, 0)`），画在玻璃形状内，因此这次改动**不影响**
  影响盒公式（不变量 5 的外扩仍只由 `grip` 决定），不需要重算模糊链外扩常量，
  也不动 `c0..c18`（宽度 / 不透明度走独立的 `BorderCb.b4`）。
  取证影响：`cmp_visual.py` 把光标物理停放在离玻璃 > `3 * mouse_radius` 处（`gate` 精确为 0），
  hover 与把手都不命中 ⇒ 四段比对里本来就没有 border，跨版本像素比对不受这次改动影响；
  差异只在肉眼验收 / 连拍（`GLASS_SHOT_EVERY_MS`）里可见。

### Removed
- `Compositor::capture_generation`（零读取点）、`draw_blocks` 的 `dirty` 死参数与死分支、
  `ErrorKind::NotSupported`（零构造点）、`.gitignore` 的 `src-tauri/*` 遗留。

## [0.4.0]

项目从**单 bin crate `liquid-glass`** 重构为 **Cargo workspace 库项目 `liquid-verre-rs`**
（package 名 `liquid-verre-rs`，**lib 名 `liquid_verre`**，版本 `0.4.0`）。
渲染路径本身没有改动：`c0..c17` 的 cbuffer 语义、模糊链外扩常量、脏区三来源并集、
自适应质量判据全部逐字保留，因此重构前的取证图仍然可比。

### Added

- **库化公共 API**（`crates/liquid-verre/src/api/`，`lib.rs` 里唯一对外 `pub use`）：
  - `Engine`：`new` / `spawn` / `start` / `run` / `join` / `shutdown` / `is_running` /
    `take_fatal_error` / `create_window` / `window` / `windows` / `window_snapshots` /
    `clear_windows` / `set_default_style` / `default_style` / `set_window_level` / `window_level` /
    `set_default_resize` / `monitors` / `snapshot` / `quality_level` / `poll_events` /
    `set_event_handler` / `request_render` / `config`。`Drop` 自动 `shutdown`，不泄漏渲染线程。
  - `GlassWindow`：`Clone + Send + Sync` 句柄，11 个 getter + 17 个写方法（几何 / 外观 /
    交互 / z 序 / 可见性 / `close`）+ `canvas_handle()`（供上层把自己的 UI 窗口贴在玻璃之上）。
  - 配置类型全部 `Default` + `Serialize`/`Deserialize`（`camelCase`）：`EngineConfig`、
    `WindowConfig`、`GlassStyle`、`TransparentParams`、`ResizeOptions`、`ResizeFeedbackOptions`、
    `CaptureConfig`、`DiagnosticsConfig`、`LogLevel`、`WindowLevel`、`ZOrder`、`GlassMode`、
    `CaptureBackend`、`Rect`。可直接用 JSON/TOML 喂（有往返单测）。
- **双模式玻璃（需求 F3）**：
  - `GlassMode::Liquid` = 完整液态玻璃，参数集 `LiquidParams`（**37 个字段**）。
  - `GlassMode::Transparent` = 纯透明玻璃，`TransparentParams` **只有 3 项**
    （`corner_radius` / `opacity` / `color`，需求 F3.2）。
  - 透明模式在 `glass_ps.hlsl` 里走**早退分支**（`c18.x > 0.5`），
    完全不采样 capture / frost / env / LUT，不做折射、色散、高光、焦散、鼠标光；
    分支在全部纹理采样之前且 `c18.x` 是 per-draw 常量，因此没有 warp 内分歧。
  - 整屏都是透明块时 **Pass A 整条跳过**（`MonitorRt::needs_backdrop` →
    `FrameInput::needs_backdrop`），启动时也不等该屏采集就绪（`wait_for_captures` 跳过），
    这是需求 F3.1 的性能余量来源。
  - 运行期可切换：`GlassWindow::set_style` / `set_liquid` / `set_transparent`；
    模式翻转会让金字塔账本 `invalidate()`，不会露出陈旧背景。
  - 透明模式映射为一组**惰性**液态参数（`LiquidParams::inert()` 底座 + 覆盖三个字段），
    因此渲染层只有一份参数结构，脏区 / 外扩 / DPI 换算逻辑完全复用。
- **边缘 / 对角缩放（需求 F2）**：
  - `core/handle.rs`：`ResizeEdges`（8 个方向常量）+ `hit_handle(rect, cursor, grip)`，
    **先角后边**；命中带伸出块外 `grip/2`，窗口区域同步外扩 `max_grip + 2`。
  - `ResizeOptions { enabled, edges, corners, grip_px, min_width, min_height, feedback }`
    可分别开关边缘与对角（`edges_active()` / `corners_active()` 都受总开关约束）。
  - `ResizeFeedbackOptions { enabled, border_width, alpha, color }`：悬停反馈是加粗白色 border。
    **直边只画到圆角切点为止**（不拐弯），**对角沿圆弧拐弯**（用 SDF 内侧距离做 band），
    实现在独立的 `shaders/border_ps.hlsl`（`GlassRenderer::draw_borders` 于 Pass B 之后叠加绘制），
    返回值再乘 `coverage` 保证不漏到形状之外。
  - `input/drag.rs::ResizeState`：8 方向缩放 + **锚定不动边**（夹到最小尺寸时固定边纹丝不动）。
  - 光标形状由 `ResizeEdges::cursor_id()` 经 `WM_SETCURSOR` 设置（8 个方向各有系统光标，有单测）。
- **层级语义（需求 F4）**：
  - `WindowLevel { AlwaysOnTop, Normal, ClickToFront, AlwaysOnBottom }` 作用于**画布窗口**
    （每块显示器一个），实现集中在 `platform/level.rs`；一律带
    `SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE`（不抢焦点、不触碰几何）。
  - `ClickToFront` = `Normal` + 点击命中玻璃块时 `HWND_TOP`（`raise_on_click`）。
  - 玻璃块之间的叠放用 `ZOrder { Front, Back, Absolute(i32), Relative(i32) }` +
    `GlassWindow::bring_to_front` / `send_to_back`。
- **全参数配置化（需求 F1）**：`playground/src/schema.rs` 是面板控件的**单一来源**
  （`liquid_fields()` 37 项 / `transparent_fields()` 3 项 / `resize_fields()` 10 项），
  前端按 `/api/schema` 动态生成控件。两个方向的单测都钉死：
  `LiquidParams` 有字段没进面板 ⇒ `every_param_is_exposed_in_the_panel` FAIL；
  面板有 key 但库里没有 ⇒ `every_liquid_field_exists_in_params` FAIL。
- **错误体系**：`Error` / `ErrorKind`（10 类，`#[non_exhaustive]`，带稳定 `as_str()`）/ `Result<T>`；
  `From<String>` / `From<&str>` / `From<windows::core::Error>` / `From<std::io::Error>` 让 `?`
  平滑收敛；底层 HRESULT 保留在 `source()`；`Error::with_kind()` 用于把 `Internal` 细化。
- **日志门面**：库内一律用 `log` 宏；提供可选的默认 stderr logger
  `install_default_logger()` / `install_default_logger_with_level(level)`（幂等，
  不覆盖上层已装的 logger）。行格式与重构前**逐字节一致**（前缀由 `target` 决定，
  `RAW_TARGETS = boot / verify / metrics / adaptive / capture / probe`）。
- **事件体系（工程需求 2）**：`EngineEvent`（`#[non_exhaustive]`，18 个变体：`Ready` /
  `MonitorsChanged` / `WindowCreated` / `WindowDestroyed` / `MoveStarted` / `MoveEnded` /
  `ResizeStarted` / `ResizeEnded` / `LevelApplied` / `CaptureBackendSelected` / `CaptureLost` /
  `CaptureRestored` / `ExclusiveFullscreen` / `QualityLevelChanged` / `Warning` / `Fatal` /
  `EventsDropped` / `Shutdown`）+ `ResizeHandle`（8 方向，`Display` 输出 kebab-case）。
  两种消费方式可并用：`set_event_handler`（渲染线程同步回调）与 `poll_events`
  （有界队列 `EVENT_QUEUE_CAP = 256`，溢出丢最旧并补一条 `EventsDropped`）。
- **线程模型**：库内单渲染线程；上层用 `GlassWindow` 写命令队列（setter 异步，延迟 ≤1 帧；
  帧首一次性消费，一帧内连发 N 条同类 setter 只有最后一条生效）、读 `Snapshot`（最终一致）。
- **构建元数据**：`crates/liquid-verre/build.rs`（零第三方依赖，自实现 SHA-256 与
  civil-from-days UTC 时间戳；拿不到 git 一律降级为 `"unknown"`，**绝不 panic**）生成
  `OUT_DIR/build_info.rs`，库暴露 `BUILD_INFO` 与 `BuildInfo::summary()`
  （VERSION / GIT_SHA / GIT_DIRTY / BUILD_TIMESTAMP_UTC / TARGET_TRIPLE / RUSTC_VERSION /
  PROFILE / SHADERS 摘要）。
- **`core/` 纯逻辑层**：`geometry`（矩形与影响盒运算的**单一来源**）、`handle`（8 方向把手）、
  `draw`（`DrawBlock` 投影块 + `ResizeFeedback`，全 `Copy`）、`style`（`StyleCache`，
  键 = `id + style_epoch + scale 位模式`，避免每帧重跑约 40 次 clamp）、`quality`、`block`、
  `manager`、`params`、`sdf`。该层**零 `windows::` 引用、零 `unsafe`**，因此全部可单测。
- **`playground/` crate**（bin `liquid-verre-playground`，`publish = false`）：
  tiny_http 控制面板（`GET /`、`/api/schema`、`/api/state`、`/api/events`；
  `POST /api/style`、`/api/mode`、`/api/resize`、`/api/window`、`/api/level`）+
  命令行参数（`--port` / `--no-browser` / `--transparent` / `--no-demo-pair` /
  `--no-default-window` / `--level`）。只用公共 API，兼作 API 的活文档。
- **`xtask/`**（零第三方依赖）：`cargo xtask build [--debug]` / `check` /
  `package [--out DIR] [--no-zip]` / `gate` / `clean`；`.cargo/config.toml` 里配了 alias。
  只做编排，阈值仍只在 `tools/gate_perf.py` 与 `cargo test` 里。
- **`scripts/*.mjs`**（Node，**零 npm 依赖**，含自实现 zip）：`_lib.mjs` / `build.mjs` /
  `check.mjs` / `gate.mjs` / `package.mjs` / `zip.mjs`，与 `cargo xtask` 等价。
- **测试**：单元测试从重构前的 25 个增加到 **库 129 个 + playground 5 个**（另有 `lib.rs` 与
  `api/engine.rs` 的 `no_run` doctest）。覆盖矩形运算、影响盒与着色器支撑窗对齐、
  把手几何与掩码打包、sanitize/NaN、样式缓存、z 序、配置 serde 往返、事件队列、
  丢帧判据、`IDLE_WAIT_MS` 取舍、构建元数据、schema 双向覆盖等。
- **`tools/cmp_visual.py` 跨版本视觉取证**：稳态帧（`GLASS_INTERNAL_SHOT_AT_MS = 2500`）、
  **四段统计**（`deep` / `interior` / `ring` / `outside`，`DEEP = 130`、`RING = 24`）、
  `CursorWatchdog`（`SetCursorPos` 物理停放 + 每 0.12 s 复位）、
  many 布局的圆角归因判据（`R_NEW = 32.0`、`R_TOL = 6.0`、`FAR_MAX_DIFF = 2`）。
- **`GLASS_DEFAULT_BLOCK=1`**：给取证脚本一块确定性的默认玻璃（库默认不再自建窗口）。
- **`VERRE_EXE` / `VERRE_OLD_EXE`**：`tools/*.py` 的被测二进制覆盖变量。

### Changed

- **`liquid-glass`（bin crate）→ `liquid-verre-rs`（lib crate）**：源码从 `src/` 迁到
  `crates/liquid-verre/src/`，着色器从 `shaders/` 迁到 `crates/liquid-verre/shaders/`，
  `build.rs` 迁到 `crates/liquid-verre/build.rs`。根 `Cargo.toml` 变成纯 `[workspace]`
  （members = `crates/liquid-verre` + `playground` + `xtask`）。
  `[profile.release]` 与重构前**逐字一致**（`opt-level=3` / `lto="fat"` / `codegen-units=1` /
  `panic="abort"` / `strip="symbols"`），否则新旧性能指标不可比。
- **`src/ui/` → `playground/`**：控制面板从库里搬出去成独立 crate，
  tiny_http / serde_json 不再是库的依赖。
- **`.ps1` → `.mjs`**：构建 / 检查 / 打包脚本从 PowerShell 改为 Node
  （`.ps1` 只在 Windows 上跑；PS 5.1 读无 BOM 的 UTF-8 会把中文注释变成乱码；
  `[CmdletBinding()]` 还会和自己的 `-Debug` 参数撞名）。
- **模块重新分层**：
  - `src/main.rs`（约 2000 行的主循环）→ `runtime/{mod,app,commands,monitor}.rs`；
  - `src/glass/` → `core/`（`params.rs` / `sdf.rs` 平移，`block.rs` / `manager.rs` 重写）；
  - `src/gfx/monitors.rs` → `platform/display.rs`，并新增 `platform/{window,level}.rs`
    （Win32 调用集中到 `platform/`）；
  - 新增 `api/`（唯一公共面）、`diag/`（`env.rs` + `metrics.rs`）；
  - `input/` 拆成 `cursor.rs`（`GetCursorPos`）与 `drag.rs`（`DragSession` = `Move` | `Resize`）。
- **`eprintln!` → `log` facade**：前缀与字段名逐字节保留（`tools/` 靠它们解析）。
- **`Result<_, String>` → `Error` / `ErrorKind` / `Result`**：靠 `From<String>` / `From<&str>`
  收敛既有代码，需要精确类别的点位显式 `Error::new` / `with_kind`。
- **控制面板的 `Arc<AtomicIsize>` 共享参数 → `Command` 队列 + `Snapshot`**：
  面板从此只是公共 API 的一个普通使用者，不再是库的一部分。
- **环境变量解析集中到 `diag/env.rs`**：主循环那批（`GLASS_METRICS` / `GLASS_MANY` /
  `GLASS_SPREAD` / `GLASS_FAR_GRID` / `GLASS_DEFAULT_BLOCK` / `GLASS_EXTREME_SPEC` /
  `GLASS_SIM_BUSY` / `GLASS_SIM_BACKDROP` / `GLASS_MONITORS` / `GLASS_VERIFY_PASS_A` /
  `GLASS_INTERNAL_SHOT` / `GLASS_CURSOR_AT` / `GLASS_ADAPTIVE`）在启动期解析一次；
  只属于某子系统构造期的（`GLASS_BACKEND` / `GLASS_SYNTH_CAP` / `GLASS_COPY_MODE` /
  `GLASS_SIM_DIRTY` / `GLASS_SKIP_*` / `GLASS_VERIFY_COPY` / `GLASS_WGC_BUFFERS` /
  `GLASS_FRAME_LATENCY` / `GLASS_ENV_EMA` / `GLASS_FROST_HALF` / `GLASS_PASS_A_HZ` /
  `GLASS_FORCE_FULL` / `GLASS_VERIFY_VALIDITY` / `GLASS_INTERNAL_SHOT_AT_MS` /
  `GLASS_SHOT_EVERY_MS` / `GLASS_BURST_OVERLAY` / `GLASS_GPU_TIMINGS`）留在原地并在
  `diag/env.rs` 的文档注释里登记。
- **圆角半径：字段 → 派生**。`Block` 不再有 `radius` 字段，改为
  `Block::effective_radius(style) = clamp(style.corner_radius() * scale, 0, 短边/2)`。
  ⚠ 这是**有意的行为变更**（需求 F1「全部参数配置化」的直接结果）：
  重构前 20 块布局里有 19 块的半径永远停在硬编码的 `28.0`，现在统一由样式派生（默认 `32.0`），
  因此跨版本比对 many 布局时**圆角带必然不同**。single 布局不受影响。
- **影响盒单一来源**：`gfx/compositor.rs` 与主循环各自手写的外扩，统一收到
  `core::geometry`：`frame_influence_px(mouse_radius, grip) = max(3R + 32, 2*grip + 16)`，
  出口是 `MonitorRt::influence_px() = frame_influence_px(R, max(grip,1)) + ceil(shadow_margin)`
  （重构前只有 `cursor_influence_px(R) + margin`，没有缩放反馈那一项）。
- **cbuffer 从 18 槽扩到 19 槽**（`CB_FLOATS = 19 * 4`）：
  **`c0..c17` 语义一字未改**，新增 `c18` = (mode, transparentOpacity, handleGrabbed, -)；
  缩放反馈的掩码/宽度/颜色**不**在玻璃 cbuffer 里，而是独立 pass 的 `BorderCb`
  （`b2` 直边掩码、`b3` 对角掩码、`b4` 宽度/不透明度、`b5` 颜色），见 `shaders/border_ps.hlsl`。
  玻璃 cbuffer 只新增 `c18.z = handleGrabbed`，用于让液态 hover 高光给 border 让位。
- **被测 EXE 路径**：`tools/*.py` 从 `target/release/liquid-glass.exe` 改为
  `target/release/liquid-verre-playground.exe`（可用 `VERRE_EXE` 覆盖）；
  `cmp_visual.py` 的两侧默认是 `tools/exe_now.exe` / `tools/exe_old.exe`
  （可用 `VERRE_EXE` / `VERRE_OLD_EXE` 覆盖）。
- **`GLASS_CURSOR_AT` 语义**：从「只设初值、主循环立刻回到真实光标」改为**全程钉死**
  （`App::pinned_cursor` → `App::read_cursor()`；窗口过程与主循环都走 `read_cursor()`，
  否则命中测试用真实光标、渲染用钉死光标会打架）。
- **库默认不自建窗口**：重构前 `main()` 启动就建一块居中玻璃；现在库不替上层决定要不要有窗口，
  需要时用 `GLASS_DEFAULT_BLOCK=1` 或 `EngineConfig.diagnostics.default_window = true`。
  默认块尺寸与重构前一致（否则历史逐位比对图不可比）。
- **默认参数对齐「用户实际长期看到的稳态值」**：`tint_opacity` `0.018 → 0.01`、
  `blur_min` `3.0 → 20.0 * 0.12`（详见 Fixed 第 1 条）。
- **文档**：`README.md` 重写为面向上层应用开发者的库文档（安装 / 最小示例 / API 速查 /
  层级语义 / 线程模型契约 / 坐标系与量纲 / 构建与阀门命令 / 目录树 / 性能表 / `GLASS_*` 全表）；
  `.trellis/spec/backend/` 全部按 workspace 结构更新；新增 `CHANGELOG.md`。

### Fixed

1. **旧面板的 `isize` 量化会篡改默认值，导致引导帧与稳态帧参数不同。**
   重构前 `src/ui/panel.rs` 的 `SharedParams::apply()` 在主循环里**每帧**把参数写回
   `app.params`，而它用 `Arc<AtomicIsize>` 存值：
   - `tint_opacity`：`(0.018 * 100.0) as isize = 1`（截断）→ 读回 `0.01`；
   - `blur_min`：面板没有滑杆，`apply()` 里按 `(blur_max * 0.12).clamp(0.5, 12.0)` 派生
     → `20 * 0.12 = 2.4`（设计值是 3.0）。

   其余 16 个面板字段的量化往返逐个核对都恰好无损，只有这两个变了。于是旧二进制存在
   **两帧不一致**：引导帧（面板 apply 之前）用 `0.018 / 3.0`，稳态帧（之后，占运行时间 99.99%+）
   用 `0.01 / 2.4`。`tools/cmp_visual.py` 实测：旧二进制自己这两帧相差 **873748 像素**、
   **最大通道差 140**。

   修复：库化后面板移出（`playground/`），不再有量化往返，引导帧与稳态帧从此**同一份参数**；
   同时把库的默认值对齐到「用户实际长期看到的稳态值」（`tint_opacity: 0.01`、
   `blur_min: 20.0 * 0.12`）。取证口径也相应改为比**稳态帧**
   （`GLASS_INTERNAL_SHOT_AT_MS = 2500`）。改这两个值 = 改视觉基线，必须同时更新
   `BASELINE.md` 与取证图。
2. **`blur_min` 必须写成表达式 `20.0 * 0.12`，不能写字面量 `2.4`。**
   f32 下 `20.0 * 0.12 = 2.3999999`（`0x40199999`），而字面量 `2.4 = 2.4000001`（`0x4019999A`），
   **差 1 ULP**。旧面板就是按 `blur_max * 0.12` 现算的，用字面量会让 `cmp_visual` 的 single
   场景多出约 **2400 个 ±1 LSB 差异像素**（实测：interior 687 + ring 1679 + outside 4，
   全部最大通道差 = 1）。`core/params.rs` 里保留了这段注释与位模式，防止被「顺手格式化」掉。
3. **旧的圆角滑杆只写「选中块」，其余块的半径永远停在硬编码值。**
   重构前主循环里是 `if target_id >= 0 && !dragging { 改选中块 } else { 改全部块 }`，
   而 `sync_selection()` 总会把 `selected_id` 兜底成 `manager.blocks().last()`，
   于是 20 块布局里有 **19 块**的 `radius` 永远停在 `Block::new` 硬编码的 `28.0`
   （只有跨屏时那条 `b.radius = 32.0 * sc` 会偶然改到它）。
   修复：半径统一由样式派生（`Block::effective_radius`），不再有「每块各存一份、
   只有选中块被更新」的状态。⚠ 这同时是**有意的行为变更**（见 Changed），
   跨版本比对 many 布局时圆角带必然不同，`cmp_visual.py` 用圆角归因判据处理这件事。
4. **`GLASS_CURSOR_AT` 过去只设初值，使「光标在玻璃内移动」那个取证用例恒为 0 差异（空转测试）。**
   重构前主循环下一帧就回到 `GetCursorPos` 的真实光标，于是「光标在 A 处」与「光标在 B 处」
   两张图其实是同一个光标位置拍的，判据永远通过、什么也没证明。
   修复：`GLASS_CURSOR_AT` 改为**全程钉死**（`App::pinned_cursor` / `App::read_cursor`），
   并且 `cmp_visual.py` 改用 `SetCursorPos` **物理停放** + 看门狗线程（对旧二进制唯一有效的办法），
   把光标停在离任何玻璃 **> 3 × mouse_radius**（默认 660 px）的地方，
   让鼠标点光的有限支撑窗 `gate` 精确为 0。
   实测踩过的坑：停放点在块内时，重构后新增的边缘 / 对角缩放 border 会让整片圆角对不上
   （最大通道差 140）。
5. **影响盒没有覆盖缩放反馈的支撑窗。** 新增的悬停 border 也是光标驱动的有限支撑效果
   （命中带宽 `grip_px * scale`），若影响盒仍只按 `3 * mouse_radius` 算，
   盒外块的缩放反馈项就不为 0，会重新出现拼接缝（不变量 5 的历史前科）。
   修复：`core::geometry` 新增 `resize_influence_px(grip) = 2*grip + 16`，
   `frame_influence_px` 取两者较大值，出口再加 `ceil(shadow_margin)`；
   单测 `influence_boxes_strictly_cover_shader_support` 钉住「严格大于着色器支撑半径」。
6. **两处手写同一常量的影响盒外扩。** 重构前 `gfx/compositor.rs` 与主循环各写了一遍，
   属于 spec 明令禁止的模式。现在统一收到 `core/geometry.rs`，
   `gfx` / `runtime` / `platform` 都只调用不重写。
7. **直边与对角反馈掩码可能同时非 0。** 若把 `TOP_LEFT` 打成 `left | top` 送进 `BorderCb.b2`，
   着色器会同时画「到切点为止的直线」和「沿圆角的弧」，两者在切点附近叠出一个亮斑
   （需求 F2.1 明确不要）。现在两种掩码在**源头**互斥
   （`to_shader_edges()` 对角返回全 0、`to_shader_corners()` 单边返回全 0），
   单测 `shader_packing_matches_semantics` 钉住。
8. **`create_window` 先 sanitize 再校验会静默吞掉编程错误。** sanitize 会把 0 宽夹到
   `MIN_SIZE = 24`，顺序反了的话「上层传了 0×100」这种明显错误会被静默改成 24×100。
   现在先校验原始输入（有限性 + 宽高 > 0，不合法直接 `Err(InvalidConfig)`）再 sanitize。
9. **快照锁中毒会让上层 API panic。** `Shared::read_snapshot` 在 `RwLock` 中毒
   （某个持有者 panic 过）时退化为 `Snapshot::default()`，getter 返回 `None` / 默认值，
   而不是把 panic 传给上层。
10. **`Engine::shutdown` 与 `Drop` 的重复退出路径。** `shutdown` 现在幂等
    （`thread.is_none()` 直接返回），`Drop` 调它，句柄在引擎退出后调用一律 no-op +
    一条 `EngineEvent::Warning`，不 panic、不返回 `Err`。

### Removed

- **`src/gfx/w_mode_compositor.rs`**：孤儿文件，没有任何模块引用它（方案 12.1 的
  「双模渲染器 / W 模式」在 BASELINE 的已知偏离清单里就是未落地项）。
- **`shaders/reduce_env.hlsl`**：死文件，从未接入管线（旧 README 里就标着
  「备用：独立 pass 版环境色探针（未接入管线）」）。环境色探针现在直接复用 `down[3]`。
- **`src/main.rs` / `src/ui/mod.rs` / `src/glass/mod.rs`**：随分层拆解消失
  （主循环进 `runtime/`，面板进 `playground/`，领域模型进 `core/`）。
- **`Block::radius` 字段**：改由 `Block::effective_radius(style)` 派生（见 Changed / Fixed 第 3 条）。
- **`GLASS_FORCE_HDR_CONV`**：旧 README 登记过，重构后的源码中已无读取点
  （HDR→SDR 由采集后端选择规则处理：HDR 输出自动走 WGC，由 DWM 完成转换）。
- **`SharedParams` 及其 `Arc<AtomicIsize>` 参数共享**：被 `Command` 队列 + `Snapshot` 取代
  （同时修掉了 Fixed 第 1 条的量化截断）。
- **构建脚本里的 PowerShell 实现**：被 `scripts/*.mjs` 与 `xtask/` 取代。

[0.4.0]: https://github.com/baiwusanyu-c/tarui-glass/releases/tag/v0.4.0
[0.4.1-unreleased]: https://github.com/baiwusanyu-c/tarui-glass/compare/v0.4.0...HEAD