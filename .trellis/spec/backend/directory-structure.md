# 目录结构与职责边界

> 重构后是 **Cargo workspace**：根 `Cargo.toml` 只有 `[workspace]`（没有 `[package]`），
> 成员 = `crates/liquid-verre` + `playground` + `xtask`。
> `[profile.release]` 与重构前逐字一致（`opt-level=3` / `lto="fat"` / `codegen-units=1` /
> `panic="abort"` / `strip="symbols"`）—— 性能阀门的历史数字是在这套 profile 下测出来的，
> **改任何一个字段都会让新旧指标不可比**。

## 1. workspace 目录树（照实际文件写）

```
liquid-verre-rs/
├─ Cargo.toml                     # [workspace] members / [workspace.package] version=0.4.0, edition=2021,
│                                 #   rust-version=1.77, license=MIT / [workspace.dependencies] windows 0.58,
│                                 #   serde, serde_json, tiny_http, log / [profile.release]
├─ Cargo.lock
├─ .cargo/config.toml             # [alias] xtask = "run --package xtask --release --"
├─ LICENSE                        # MIT
│
├─ crates/liquid-verre/           # ★ 库本体：package `liquid-verre-rs`，lib `liquid_verre`
│  ├─ Cargo.toml                  # deps: windows / serde / log；dev-deps: serde_json；build = "build.rs"；
│  │                              #   readme = "README.md"、keywords/categories 已填；repository 仍是**注释占位**
│  │                              #   （等仓库 URL 定下来再填；缺失时 cargo package 只告警不失败）
│  ├─ README.md                   # crate 自带短 README：`cargo package` 要求 readme 在 **crate 目录内**
│  │                              #   （workspace 根的 README 不会被打进 .crate）；正文指回仓库根的完整文档
│  ├─ build.rs                    # 零第三方依赖（自实现 SHA-256 + civil_from_days UTC 时间戳）；
│  │                              #   生成 OUT_DIR/build_info.rs；拿不到 git 一律降级为 "unknown"，绝不 panic
│  ├─ shaders/                    # HLSL 源，运行期由 gfx/shaders.rs 的 include_str! 内嵌
│  │  ├─ glass_vs.hlsl            # 逐块矩形 -> 裁剪空间；SV_Position = 画布像素坐标
│  │  ├─ glass_ps.hlsl            # SDF + Snell 折射 + 色散 + Fresnel + 焦散 + 鼠标点光
│  │  │                           #   + 透明模式早退分支（c18.x）；缩放反馈 border 不在本文件
│  │  ├─ blur_vs.hlsl             # 全屏三角形 VS（模糊链共用）
│  │  ├─ blur_down.hlsl           # 4-tap 角点下采样
│  │  ├─ blur_up.hlsl             # 8-tap tent 上采样
│  │  └─ env_ema.hlsl             # 1/16 环境色探针时域平滑（方案 12.6 / R-20）
│  └─ src/
│     ├─ lib.rs                   # crate 门面：#![warn(missing_docs)] + #![warn(clippy::all)]；
│     │                           #   `pub mod api` + 全部对外 `pub use`（唯一 re-export 点）
│     ├─ error.rs                 # Error / ErrorKind（10 类）/ Result + From<String|&str|windows::core::Error|io::Error>
│     ├─ logging.rs               # log facade；可选默认 stderr logger；RAW_TARGETS 白名单
│     ├─ build_info.rs            # include!(OUT_DIR/build_info.rs) → BUILD_INFO / BuildInfo::summary()
│     │
│     ├─ api/                     # ★★ 唯一对上层暴露的模块（公共面）
│     │  ├─ mod.rs                # pub use 汇总；LiquidParams / Profile / Shape 从 core::params 重导出
│     │  ├─ config.rs             # EngineConfig / WindowConfig / GlassStyle / GlassMode / TransparentParams /
│     │  │                        #   ResizeOptions / ResizeFeedbackOptions / WindowLevel / ZOrder /
│     │  │                        #   CaptureConfig / CaptureBackend / DiagnosticsConfig / LogLevel /
│     │  │                        #   type Rect = [f32;4] / clamp_finite()（NaN 安全 clamp）
│     │  ├─ engine.rs             # Engine：生命周期(new/spawn/start/run/join/shutdown/is_running) +
│     │  │                        #   窗口工厂(create_window/window/windows/window_snapshots/clear_windows) +
│     │  │                        #   全局配置(default_style/window_level/default_resize) +
│     │  │                        #   观测(monitors/snapshot/quality_level/poll_events/take_fatal_error/config)
│     │  ├─ handle.rs             # GlassWindow 句柄（Clone+Send+Sync）、Command 枚举、Shared（命令队列 +
│     │  │                        #   快照 RwLock + 事件队列 + 渲染请求位 + fatal 槽）、
│     │  │                        #   Snapshot / WindowSnapshot / MonitorSnapshot
│     │  └─ event.rs              # EngineEvent（18 变体）/ EventHandler / ResizeHandle（8 方向）/ EVENT_QUEUE_CAP=256
│     │
│     ├─ core/                    # ★ 纯逻辑领域模型：**零 `windows::` 引用、零 `unsafe`**，因此全部可单测
│     │  ├─ params.rs             # LiquidParams（37 字段）+ Profile + Shape + sanitize + scaled(DPI) + inert()
│     │  ├─ block.rs              # Block（逻辑单位）+ MIN_SIZE=24 / MAX_SIZE=8000 + clamp + effective_radius
│     │  │                        #   occluded / occl_streak + occlusion_step（遮挡迟滞状态机：置位需连续
│     │  │                        #     OCCLUSION_CONFIRM_POLLS=2 轮命中、**解除只需 1 轮**；streak 到阈值即饱和，不许溢出 i8）
│     │  ├─ manager.rs            # 块集合 / z 序（Front/Back/Absolute/Relative）/ hit_test / hit_resize / 跨屏归属
│     │  ├─ handle.rs             # ResizeEdges（8 方向常量）+ hit_handle（先角后边）+ to_shader_edges/corners
│     │  │                        #   （直边与对角掩码在**源头**互斥）+ cursor_id（WM_SETCURSOR）
│     │  ├─ draw.rs               # DrawBlock（按 DPI 展开后的渲染投影块，Copy）+ ResizeFeedback + draw_rect
│     │  ├─ style.rs              # StyleCache：以 (id, style_epoch, scale 位模式) 为键缓存展开后的参数
│     │  ├─ geometry.rs           # ★ 矩形 / 影响盒运算的**单一来源**：cursor_influence_px / resize_influence_px /
│     │  │                        #   frame_influence_px / rect_{contains,union,hit,inflate,clamp,area} / rects_bbox
│     │  │                        #   / rect_is_empty_i64（「矩形退化」判据的**唯一**出处，i32 版委托给它；platform 侧
│     │  │                        #     全程 i64 也调它 —— 不许再手写 `x1 <= x0 || y1 <= y0`，即使理由是「防溢出」：
│     │  │                        #     纯比较本来就不会溢出，widen 只是纪律一致性）
│     │  │                        #   / rect_fully_covered（矩形并集**完整覆盖**判定：分割法差集 + 单 cover 快路径
│     │  │                        #     + 碎片上限 OPEN_RECT_CAP=256，超限即保守退出成「未遮挡」= 多渲染）
│     │  ├─ quality.rs            # QualityCtrl：自适应质量控制器（level 0–4，判据只看 own_us）
│     │  └─ sdf.rs                # build_refraction_lut（256 项）+ clamp_for_block + is_lod3 + LUT_SIZE
│     │
│     ├─ runtime/                 # 引擎主循环与内部状态机（**只在渲染线程存在**，无锁）
│     │  ├─ mod.rs                # run()：RoInitialize / PerMonitorV2 / 枚举显示器 / 建设备与画布 /
│     │  │                        #   seed_diagnostic_blocks / 主循环（逐帧顺序见 rendering-pipeline.md）
│     │  ├─ app.rs                # App 状态机 + FrameCtx + SpawnSpec + collect_monitor_blocks /
│     │  │                        #   render_monitor / publish_snapshot / rebuild_monitors / wait_for_captures /
│     │  │                        #   sync_block_monitor；read_cursor()（钉死光标优先）
│     │  ├─ commands.rs           # Command 在渲染线程的消费点：**帧首一次性** drain + apply（输入合并）
│     │  ├─ occlusion.rs          # 逐块遮挡剔除的**编排层**（8 Hz）：`Block::occluded` 的唯一写点。
│     │                           #   只决定「何时扫 / 扫哪块屏 / 结论写回谁 / 何时打日志」，
│     │                           #   几何判定走 core::geometry、z 序枚举走 platform::occlusion，
│     │                           #   本模块**一行 Win32 与一行矩形运算都不自己写**
│     │  └─ monitor.rs            # MonitorRt：一块屏 = 画布窗口 + 交换链 + 采集会话 + 金字塔；
│     │                           #   region_margin() / influence_px()（影响盒单一出口）
│     │
│     ├─ platform/                # ★ Win32 集中地（`unsafe` 密度最高）
│     │  ├─ window.rs             # 窗口类 LiquidVerreCanvas / create_canvas / 窗口过程 / WM_NCHITTEST 命中测试 /
│     │  │                        #   SetWindowRgn 区域 / 点击穿透 / apply_visibility / set_cursor
│     │  │                        #   is_own_window（**零堆分配**，委托纯函数 class_name_is_canvas 直接比宽字符常量；
│     │  │                        #     它被 platform/occlusion.rs 的 EnumWindows 回调对桌面上**每个**窗口调一次，分配即违规。
│     │  │                        #     误判后果被停产门 A 放大：把自家画布认成独占全屏应用 ⇒ 整屏停采 ⇒ 玻璃永久消失且不报错）
│     │  ├─ level.rs              # WindowLevel → WS_EX_TOPMOST + SetWindowPos（HWND_TOPMOST/NOTOPMOST/TOP/BOTTOM），
│     │  │                        #   一律 SWP_NOACTIVATE|SWP_NOMOVE|SWP_NOSIZE；raise_on_click
│     │  ├─ display.rs            # 显示器枚举（物理像素 + 每屏 DPI + scale）/ refresh_hz / on_battery /
│     │                           #   fullscreen_app_on（独占全屏检测，方案 R-16；返回 Option<HWND>）
│     │                           #   + is_fullscreen_shape / is_non_candidate_foreground（两条**纯**判据，
│     │                           #     可单测：形状 + cloaked；身份排除含 Progman 桌面本身）
│     │                           #   + dwm_cloaked（**全仓唯一** DWM 调用点，返回 Option<bool>：
│     │                           #     两个调用方对「查询失败」的保守方向相反，不许在 helper 里写死）
│     │                           #   + window_desc（class/title/rect，**只在翻转沿调用**，会分配 String）
│     │  └─ occlusion.rs          # EnumWindows z 序扫描：本屏画布**之上**的不透明窗口矩形。
│     │                           #   回调里的关卡顺序是**性能契约**（便宜的在前，唯一跨进程的
│     │                           #   DwmGetWindowAttribute(DWMWA_CLOAKED) 在最后），不要重排；
│     │                           #   一切「不确定」判**不遮挡**；探针 HRGN 必须 DeleteObject；
│     │                           #   回调必须**结构上不可能 panic**（release 是 panic=abort，
│     │                           #   catch_unwind 无意义）：零索引、零 unwrap、整数一律 widen 到 i64
│     │
│     ├─ gfx/                     # D3D11 / DXGI / DirectComposition
│     │  ├─ device.rs             # 设备 / DXGI 工厂 / SetMaximumFrameLatency（GLASS_FRAME_LATENCY，默认 1）
│     │  ├─ shaders.rs            # include_str! + D3DCompile + RT/SRV/cbuffer/sampler/blend 工厂
│     │  ├─ swapchain.rs          # CreateSwapChainForComposition（premultiplied alpha，flip 模型）
│     │  ├─ dcomp.rs              # DComp device / target / visual / Commit
│     │  ├─ compositor.rs         # 每屏画布：FrameInput、脏区推导与并集、金字塔有效性账本、
│     │  │                        #   Clear → Pass A 调度（全透明屏整条跳过 / 限频）→ Pass B → Present → Commit、
│     │  │                        #   verify_validity 自检、取证落盘（BMP）
│     │  ├─ pyramid.rs            # Pass A：下采样×4（DOWN_GROW=[14,12,8,0]）、frost ping-pong tent×4
│     │  │                        #   （外扩 32/24/16/8）、env EMA、frost 半分辨率档、scissor 幂等
│     │  ├─ glass_renderer.rs     # Pass B：per-block CB 打包（c0..c18，见 rendering-pipeline §3.4）、
│     │  │                           #   border 叠加 pass（draw_borders / BorderCb，见 §3.4b）、
│     │  │                        #   逐块 LUT 缓存（MAX_LUTS=8，键 = ior 位模式 | profile）、z 序绘制
│     │  └─ gpu_timer.rs          # D3D11 timestamp query（GLASS_GPU_TIMINGS=1 才创建；**禁止进闭环判据**）
│     │
│     ├─ capture/                 # 桌面采集（细节不得泄漏到 gfx/，一律经 Capture 封装）
│     │  ├─ mod.rs                # Capture 统一封装 + for_monitor() 后端选择（HDR 规则）+ 确定性合成源（SYNTH）
│     │  ├─ dda.rs                # DXGI Desktop Duplication：脏/移动矩形、sanitize_rect、局部拷贝
│     │  │                        #   （CopyMode multi|union|full，MAX_DIRTY_RECTS=24）、失效退避、verify 回读
│     │  └─ wgc.rs                # Windows.Graphics.Capture：CreateFreeThreaded 帧池、B8G8R8A8、
│     │                           #   帧生命周期（拷完立刻 Close，绝不跨帧持有）
│     │
│     ├─ input/
│     │  ├─ cursor.rs             # GetCursorPos 轮询（零全局 hook，方案 6.3）
│     │  └─ drag.rs               # DragSession = Move(MoveState) | Resize(ResizeState)：速度 EMA(0.75/0.25)、
│     │                           #   8 方向缩放、锚定不动边、逻辑单位换算（跨屏安全）
│     │
│     └─ diag/                    # 诊断层：**不属于对外契约**，全关时行为与「诊断代码不存在」逐位一致
│        ├─ env.rs                # EnvOverrides::load()：主循环那批 GLASS_* 的**唯一**解析点（启动期一次）
│        └─ metrics.rs            # Metrics + report_metrics()：5 秒墙钟窗口的 `[metrics]` 行
│
├─ playground/                    # dev 控制面板（bin `liquid-verre-playground`，publish=false）
│  │                              # ★ **不是库的业务代码**：只用 `liquid_verre` 的公共 API，没有一个 pub(crate) 后门，
│  │                              #   因此兼作 API 的活文档。重构前这里是 `src/ui/`（tiny_http 混在渲染依赖图里）
│  ├─ Cargo.toml                  # deps: liquid-verre-rs(path) / serde / serde_json / tiny_http / log
│  └─ src/
│     ├─ main.rs                  # Engine::spawn + 事件轮询打印 + 演示窗口对（liquid/transparent）；
│     │                           #   取证/计量模式下（GLASS_* 布局或诊断开关）一律不加演示窗口
│     ├─ panel.rs                 # tiny_http：GET / 、/api/schema 、/api/state 、/api/events ；
│     │                           #   POST /api/style 、/api/mode 、/api/resize 、/api/window 、/api/level
│     ├─ schema.rs                # ★ 面板控件的**单一来源**：liquid_fields(37) / transparent_fields(3) /
│     │                           #   resize_fields(10)；单测双向钉住「LiquidParams 有字段没进面板就 FAIL」
│     └─ web/index.html           # 前端页面，按 /api/schema 动态生成控件（不手写第二份字段表）
│
├─ xtask/                         # `cargo xtask <cmd>`：零第三方依赖（纯 std）的编排层
│  └─ src/main.rs                 # build[--debug] / check / package[--out DIR][--no-zip] / gate[-- args] / clean
│                                 #   判定逻辑留在 tools/gate_perf.py 与 cargo test，避免两处阈值漂移；
│                                 #   压缩委托 scripts/zip.mjs（NODE 环境变量可换可执行名）
│
├─ scripts/                       # Node 构建脚本：**零 npm 依赖**（含自实现 zip），跨平台、固定 UTF-8
│  ├─ _lib.mjs                    # REPO_ROOT / run() / pythonExe() / crateVersion() / targetTriple() /
│  │                              #   gitInfo() / profileDir() / createZip()
│  ├─ build.mjs                   # = cargo build --workspace [--release] [--only <pkg>]
│  ├─ check.mjs                   # = fmt --check + clippy -D warnings + test --release（三步全跑完再汇总）
│  ├─ gate.mjs                    # = 先构建再调 tools/gate_perf.py，透传退出码
│  ├─ package.mjs                 # = 构建 + 收集产物到 dist/ + build-info.json + zip
│  ├─ zip.mjs                     # = node scripts/zip.mjs <sourceDir> <outZip>
│  └─ release.mjs                 # 本地发版助手：工作区必须干净 → 算目标版本（--bump patch|minor|major
│                                 #   或 --version x.y.z，--tag 覆盖 tag 名）→ node scripts/check.mjs →
│                                 #   cargo package -p liquid-verre-rs --list --allow-dirty 干跑 →
│                                 #   改根 Cargo.toml 的 version + CHANGELOG 的 `## [x.y.z-unreleased]` 标题 →
│                                 #   打印后续 git 命令。**不 commit / tag / push**（项目约定）；
│                                 #   --dry-run 只打印不落盘，--publish 才在本地 cargo publish
│
├─ tools/                         # 性能探针与逐像素取证（Python）；被测 EXE 默认
│  │                              #   target/release/liquid-verre-playground.exe，可用 VERRE_EXE / VERRE_OLD_EXE 覆盖
│  ├─ perf_probe.py  gate_perf.py                # [metrics] 行解析 → 指标 / 硬门槛
│  ├─ cmp_visual.py                              # 跨版本稳态帧四段取证（deep/interior/ring/outside）
│  ├─ verify_dirty.py verify_mouse_isolation.py verify_validity.py verify_adaptive.py
│  ├─ verify_crossscreen.py verify_env_ema.py verify_all.py
│  ├─ repro_tremble.py ab_latency.py ab_memory.py ab_influence.py
│  └─ attribute.py measure_frost_half.py measure_pass_a_hz.py experiments.py repeat_exp.py
│
├─ .github/workflows/             # CI / 发布流水线：只做编排，不含任何阈值（阈值仍在 tools/ 与 cargo test）
│  ├─ ci.yml                      # push / PR（windows-latest）：fmt --check + clippy -D warnings +
│  │                              #   test --release --workspace + cargo package -p liquid-verre-rs --list 干跑。
│  │                              #   **刻意不跑**像素取证与性能阀门（依赖 DDA 桌面复制、真实 vsync 与
│  │                              #   可交互桌面会话；口径见 measurement-and-gates.md §1）
│  └─ release.yml                 # `v*` tag / workflow_dispatch：windows 上 check.mjs + package.mjs → artifact
│                                 #   → ubuntu 上从 CHANGELOG 抽 `## [x.y.z]` 段做 notes 建 GitHub Release
│                                 #   （挂 dist/*.zip + dist/*/build-info.json，版本含 `-` 自动 prerelease）
│                                 #   → 配了 secret CARGO_REGISTRY_TOKEN 才 cargo publish --no-verify
│
├─ .trellis/spec/                 # 本目录（backend/ + guides/）
├─ README.md                      # 面向上层应用开发者
├─ CHANGELOG.md                   # Keep a Changelog 风格
├─ BASELINE.md                    # 重构**前**基线快照（数字仍是旧口径）
├─ performance_report.md          # 第一~十四轮取证与决策记录
└─ AGENTS.md / CLAUDE.md          # AI 协作约定（AGENTS.md 含 Trellis 托管块，块内勿改）
```

## 2. 分层与依赖方向

```
上层应用 ──► api/  ──► runtime/ ──► platform/ ──► Win32
                 │            ├───► gfx/      ──► D3D11 / DXGI / DComp
                 │            ├───► capture/  ──► DDA / WGC
                 │            └───► input/
                 └──► core/（纯逻辑，谁都能依赖它，它谁都不依赖）
                      diag/（横切：env 解析 + metrics 输出）
```

依赖只能自上而下。`core/` 是叶子：不依赖 `windows::`、不依赖 `gfx/`/`capture/`/`platform/`，
因此它是唯一能在没有 GPU 的机器上跑单测的一层（现状：`core/` 下 **0 个 `windows::` 引用、0 个 `unsafe`**）。

## 3. 边界规则（硬约束）

1. **`api/` 是唯一公共面。** `lib.rs` 只 `pub mod api`，其余模块一律 `pub(crate)`；
   对外类型全部从 `api` 重导出（`LiquidParams` / `Profile` / `Shape` 例外：它们定义在
   `core::params`，由 `api::mod.rs` 与 `lib.rs` 重导出）。新增公共项 = 改 `api/` + `lib.rs` 的 `pub use`，
   不允许把内部模块直接 `pub` 出去图省事。
2. **`core/` 不得依赖 `windows::`。** 需要平台信息（DPI、光标、显示器几何）时由 `runtime/` 取值后
   **作为参数传入**（例：`Block::effective_radius(&self, style)`、`DragSession::apply(cursor, now, scale, min_size)`）。
   这样 `core/` 的每一条规则都能被 `cargo test --release --workspace` 覆盖。
3. **`unsafe` 只允许出现在 Win32 / GPU / COM 边界**：`platform/`（24 处）、`gfx/`（69 处）、
   `capture/`（32 处），以及 `runtime/`（13 处：`RoInitialize`、`SetProcessDpiAwarenessContext`、
   消息泵 `PeekMessageW/TranslateMessage/DispatchMessageW`、`GWLP_USERDATA` 绑定与解绑、`DestroyWindow`）
   与 `input/cursor.rs`（1 处 `GetCursorPos`）。
   **`api/` 与 `core/` 必须保持 0 个 `unsafe`**（现状已如此，改动后仍须为 0）。
   每个 `unsafe` 块上方要写清「不变量由谁保证」。
4. **上层不得直接触碰 GPU 资源。** 一切改动走 `api::handle::Command` 队列：上层 setter 推命令 +
   置渲染请求位，渲染线程**帧首一次性** `drain_into` 消费（`runtime/commands.rs::apply`）。
   一帧内连发 100 条 `set_style` 只有最后一条生效，不会渲染 100 次（方案 12.2.3 输入合并）。
   读路径走 `Snapshot`（`RwLock`，最终一致 ≤1 帧），**不加锁竞争渲染线程**。
5. **采集细节不得泄漏到 `gfx/`。** `gfx/compositor.rs` 只认 `ID3D11Texture2D` + 脏矩形，
   后端差异（DDA 的 `AcquireNextFrame` 阻塞语义、WGC 的帧池生命周期）全部封在 `capture/`。
6. **`gfx/` 原则上不读环境变量**（诊断开关由 `diag::env` 统一解析后传入）。
   **已登记的例外**（`diag/env.rs` 文档注释里点名）：`GLASS_COPY_MODE` / `GLASS_SIM_DIRTY` /
   `GLASS_SKIP_COPY` / `GLASS_SKIP_DIRTY` / `GLASS_VERIFY_COPY` 在 `capture/dda.rs`，
   `GLASS_ENV_EMA` / `GLASS_FROST_HALF` 在 `gfx/pyramid.rs`，
   `GLASS_SHOT_EVERY_MS` / `GLASS_SKIP_*` / `GLASS_FORCE_FULL` / `GLASS_VERIFY_VALIDITY` /
   `GLASS_INTERNAL_SHOT(_AT_MS)` / `GLASS_PASS_A_HZ` / `GLASS_GPU_TIMINGS` / `GLASS_BURST_OVERLAY`
   在 `gfx/compositor.rs`，`GLASS_SYNTH_CAP` / `GLASS_BACKEND` 在 `capture/mod.rs`，
   `GLASS_FRAME_LATENCY` 在 `gfx/device.rs`，`GLASS_WGC_BUFFERS` 在 `capture/wgc.rs`。
   这些**只在构造期读一次**、不进热路径，因此留在原地比强行搬进 `diag::env` 更清晰。
   新增开关时：主循环要用的进 `diag/env.rs`；只属于某个子系统构造期的可以留在原地，但必须登记进
   `diag/env.rs` 的文档注释与 README 的 `GLASS_*` 表。
7. **面板 / 调试类代码不得进库。** 浏览器控制面板、HTTP 服务器、演示布局全在 `playground/`；
   库里只保留 `tools/` 取证脚本依赖的**确定性布局**（`seed_diagnostic_blocks`：
   `GLASS_MANY` / `GLASS_FAR_GRID` / `GLASS_SPREAD` / `GLASS_DEFAULT_BLOCK`），
   且正常路径下一块都不建。
8. **单一来源，禁止两处手写同一常量。** 矩形与影响盒运算只在 `core/geometry.rs`；
   8 方向把手判定只在 `core/handle.rs`；圆角有效半径只在 `core/block.rs::effective_radius`；
   面板字段表只在 `playground/src/schema.rs`。历史上「`gfx/compositor.rs` 与 `main.rs` 各写一遍
   影响盒外扩」直接导致过拼接缝（BASELINE 不变量 5）。
9. **`xtask/` 与 `scripts/` 只做编排，不做判定。** 阈值只在 `tools/gate_perf.py`（性能）与
   `cargo test`（功能）里各存一份；两条路径（`cargo xtask ...` 与 `node scripts/*.mjs`）必须等价，
   改一边就要改另一边。
10. **新增 `LiquidParams` 字段必须同时进 `playground/src/schema.rs::liquid_fields()`**，
    否则 `every_param_is_exposed_in_the_panel` 单测直接 FAIL（需求 F1 的反向保证）。
    反过来 schema 里写了 `LiquidParams` 没有的 key，`every_liquid_field_exists_in_params` 也会 FAIL。
11. **新增「窗口级」配置字段必须走完整条链**（`WindowConfig.movable`，0.4.1，是现成样板）：
    `api/config.rs::WindowConfig` →（`runtime/app.rs::SpawnSpec::new`）→ `core/block.rs::Block`
    → `api/handle.rs::Command` 新变体 + `GlassWindow` setter →（`runtime/commands.rs::apply_one`）
    → 需要参与渲染 / 命中判定时再投影到 `core/draw.rs::DrawBlock`
    （`runtime/app.rs::collect_monitor_blocks`；`movable` 就是被 `platform/window.rs::on_button_down`
    这样消费掉的）→ 需要被上层读回时进 `api/handle.rs::WindowSnapshot` +
    `runtime/app.rs::publish_snapshot`，**并在 `apply_one` 里置 `app.snapshot_dirty = true`**
    （`publish_snapshot` 的变更判据是 `snapshot_dirty || monitors_changed || snapshot_key_next != snapshot_key`，
    而 `snapshot_key` 只编码 rect / z_order / `visible*2+interactive`，`movable` **不在里面**，
    只能靠 `snapshot_dirty` 发布）→ 面板要暴露就再加 `playground/src/panel.rs` 的 action
    与 `web/index.html` 的控件。
    少一环的症状是「设了不生效」或「面板永远读到默认值」，而且**不会有任何编译错误**。
    判据提示：`apply_one` 的返回值表示「是否需要重画」，纯状态字段应返回 `false` 并只置
    `snapshot_dirty`（`SetMovable` / `SetInteractive` 如此；`SetVisible` 例外，它改采集门控要返回 `true`）。

## 4. 各层职责一句话

| 层 | 职责 | 明确**不**做什么 |
|---|---|---|
| `api/` | 对外类型与句柄；命令入队、快照出队；配置 sanitize | 不含像素/几何数学，不碰 Win32 |
| `core/` | 纯逻辑：参数、块、z 序、把手几何、投影、质量档位、LUT、矩形运算 | 不 `use windows::`，不读环境变量，不做 I/O |
| `runtime/` | 主循环调度、帧序、门控、命令消费、快照发布、诊断布局 | 不写像素逻辑，不重建模糊核 |
| `platform/` | 全部 Win32：窗口类/过程、层级、区域、命中测试、显示器与电源查询 | 不决定渲染内容 |
| `gfx/` | D3D11/DXGI/DComp：设备、着色器、金字塔、逐块绘制、脏区账本、取证落盘 | 不认采集后端差异，不做输入判定 |
| `capture/` | 桌面采集：后端选择、脏矩形、局部拷贝、失效退避、合成源 | 不碰玻璃渲染 |
| `input/` | 光标轮询与拖拽/缩放状态机（逻辑单位） | 不做命中测试的把手阈值（在 `core/handle.rs`） |
| `diag/` | `GLASS_*` 解析（主循环那批）+ `[metrics]` 行 | 不参与渲染决策；全关时零影响 |
| `playground/` | dev 控制面板；公共 API 的活文档 | 不是发布物（`publish = false`），不得被库反向依赖 |
| `xtask/` `scripts/` | 构建/检查/打包/阀门编排；`release.mjs` 另管版本号与 CHANGELOG 标题 | 不存阈值，不做判定；**不 commit / tag / push** |
| `.github/workflows/` | CI（fmt + clippy + test + `cargo package --list` 干跑）与发布（tag → Release → 可选 crates.io） | 不跑像素取证与性能阀门（runner 无交互桌面，数字无意义），不存阈值 |
| `tools/` | 性能探针与逐像素取证 | 产物不入库（`.gitignore`） |