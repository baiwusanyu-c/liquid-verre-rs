# Implement —— 执行计划

> 每个阶段结束都必须：`cargo build --release --workspace` + `cargo test --release --workspace` 全绿。
> 阶段 5/6/7 涉及视觉，额外跑像素取证。性能结论只在机器健康窗口给出
> （`rel_max<5ms`、`acq_max<25ms`），用轮转 A/B + 中位数。

## 阶段 0：基线留档
- [x] `cargo build --release` 通过（15.8 s）
- [x] `cargo test --release` 25 passed
- [ ] 记录 `gate_perf.py` 基线数字到本文件末尾（机器健康时）

## 阶段 1：workspace 骨架 + 文件搬迁（零行为变化）
- [ ] 根 `Cargo.toml` 改为 `[workspace]`；新建 `crates/liquid-verre/Cargo.toml`
      （package `liquid-verre-rs`，`[lib] name="liquid_verre" path="src/lib.rs"`）
- [ ] `git mv src crates/liquid-verre/src`；`git mv shaders crates/liquid-verre/shaders`；
      `git mv build.rs crates/liquid-verre/build.rs`
- [ ] `src/main.rs` 暂存为 `src/bin/playground_main.rs`（阶段 2 再拆），
      保证 `cargo build` 仍能出可执行文件用于回归
- [ ] `playground/` crate 骨架；`xtask/` crate 骨架；`.cargo/config.toml` alias
- [ ] 验证：build + test 绿

## 阶段 2：main.rs 拆分（零行为变化）
- [ ] `runtime/mod.rs`（App + 主循环）、`runtime/frame.rs`、`core/quality.rs`（QualityCtrl）、
      `platform/window.rs`（wndproc/region/visibility）、`platform/display.rs`
      （refresh_hz/on_battery/fullscreen_app_on）、`diag/metrics.rs`（Metrics/report_metrics/pct）、
      `diag/env.rs`（GLASS_* 集中解析）
- [ ] `gfx/compositor.rs` 的矩形自由函数抽到 `core/geometry.rs`（消除重复实现）
- [ ] `lib.rs` 建立 `pub(crate)` 模块树；`api/` 先只放最小 `Engine`（内部就是原 App）
- [ ] 验证：build + test 绿；`GLASS_METRICS=1` 冒烟一行 metrics 正常

## 阶段 3：错误 + 日志（stderr 行格式不变）
- [ ] `error.rs`（ErrorKind/Error/Result），`Result<_, String>` → `Result<_, Error>` 全量替换
- [ ] `logging.rs`：`install_default_logger()`，严格保留 `[boot]/[info]/[warn]/[verify]/metrics` 前缀
- [ ] 所有 `eprintln!` → `log::{info,warn,error,debug}!`
- [ ] `build.rs` 生成 `build_info.rs`；`lib.rs` 暴露 `build_info()`
- [ ] 验证：build + test 绿；`python tools/perf_probe.py --case single-idle --seconds 3` 能解析

## 阶段 4：配置 API + 逐块 style（默认路径逐位不变）
- [ ] `api/config.rs`：EngineConfig/WindowConfig/GlassStyle/GlassMode/ResizeOptions/WindowLevel/CaptureConfig
- [ ] `core/style.rs`：GlassStyle ↔ Params 映射 + sanitize
- [ ] `core/block.rs` 增 `style: GlassStyle`；`core/manager.rs` 支持逐块参数
- [ ] `gfx/glass_renderer.rs`：有界 LUT 缓存（key = ior 位模式 + profile）
- [ ] `api/handle.rs` + `runtime/commands.rs`：命令队列
- [ ] 验证：build + test；`GLASS_INTERNAL_SHOT` 截图与基线 `cmp_visual.py` 0 差异

## 阶段 5：双模式（F3）
- [ ] `glass_ps.hlsl`：`c22 = (mode, opacity, pad, pad)`、`c23 = color rgba`；透明早退分支
- [ ] `CB_FLOATS` 18 → 24（预留 resize 槽位）
- [ ] `Compositor::render` 增 `needs_backdrop`；全透明屏跳过 Pass A
- [ ] `runtime`：`needs_backdrop` 计算 + 0→1 跳变 `invalidate()`；采集保活判据
- [ ] 单测：模式映射、透明参数只受 3 项影响、Pass A 跳过判据
- [ ] 验证：液态模式截图与基线 0 差异；透明模式截图人工核对；gate_perf 新增 transparent 场景

## 阶段 6：8 handle 缩放 + border 反馈（F2）
- [ ] `core/params.rs`：`resize_influence_px(grip)`；影响盒取 max
- [ ] `input/resize.rs`：ResizeEdges + handle 命中 + 锚定几何 + 速度 EMA
- [ ] `core/manager.rs`：`hit_handle(x,y,grip) -> Option<ResizeEdges>`（先角后边）
- [ ] `platform/window.rs`：region 外扩量 = max(shadow_margin, grip+margin)；WM_SETCURSOR 光标反馈
- [ ] `glass_ps.hlsl`：c18/c19/c20/c21 直边 mask（切点截断）+ 对角 mask（沿弧）+ premultiplied 合成
- [ ] `gfx/glass_renderer.rs`：逐块打包 c18..c21（仅 hover 块非 0）
- [ ] 单测（≥10）：8 方向命中、MIN_SIZE 锚定、跨屏换算、禁用开关、影响盒覆盖
- [ ] 验证：`GLASS_SHOT_EVERY_MS` 连拍 → 直边/对角 border 像素证据；
      `verify_mouse_isolation.py` 仍 0 差异；`verify_dirty.py` 连跑 ≥3 次

## 阶段 7：层级（F4）
- [ ] `platform/level.rs`：WindowLevel → TOPMOST / Normal / ClickToFront / AlwaysOnBottom
- [ ] `Manager`：send_to_back / set_z / raise；z 序变化触发 geometry_changed
- [ ] 单测：z 序操作、level 枚举序列化
- [ ] 验证：手工核对 4 种层级 + 截图

## 阶段 8：playground 全参数面板（F1/E5）
- [ ] `playground/src/panel.rs`：HTTP + JSON，读写 **全量** LiquidParams / TransparentParams
- [ ] `playground/src/web/index.html`：分组折叠面板（几何/折射/毛玻璃/染色/光照/鼠标/阴影/缩放/层级/模式）
- [ ] 模式切换、resize 开关、层级切换、逐块 style 编辑
- [ ] 验证：面板每一项都能改到位（逐项冒烟）

## 阶段 9：测试 / 工具 / 文档（E6/E7/F5）
- [ ] `tools/*.py` EXE 路径 → `target/release/liquid-verre-playground.exe`（支持 `VERRE_EXE` 覆盖）
- [ ] `gate_perf.py` 增 transparent 场景（阈值与液态一致）
- [ ] `crates/liquid-verre/tests/`：API 集成测试（config 往返、错误类型、句柄命令）
- [ ] README.md / CLAUDE.md / AGENTS.md / BASELINE.md / CHANGELOG.md 重写
- [ ] `.trellis/spec/backend/*` 更新（directory-structure / rendering-pipeline / error-handling /
      logging-guidelines / quality-guidelines / measurement-and-gates）
- [ ] `.trellis/spec/guides/graphics-change-checklist.md` 补充新增不变量

## 阶段 10：重命名 + 收尾（E8）
- [ ] `cargo xtask check` 全绿、`cargo xtask package` 出 dist
- [ ] `Rename-Item D:\github-project\tarui-glass → liquid-verre-rs`
- [ ] 新路径下重跑 build + test + gate_perf
- [ ] 提醒用户在 Codex 中重新打开新目录

## 验证命令速查
```powershell
cargo build --release --workspace
cargo test  --release --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo xtask check
cargo xtask package
python tools/gate_perf.py
python tools/verify_dirty.py        # 连跑 >= 3 次
python tools/verify_mouse_isolation.py
python tools/cmp_visual.py <base.bmp> <new.bmp>
```

## 回滚点
- tag `baseline/pre-refactor`（= `82fc311`）、分支 `backup/pre-refactor-baseline`
- 重构起点 `4dca041`（基线 + 阀门，无行为变化）
- 本次工作分支：`codex/liquid-verre-rs-refactor`

## 基线数字（阶段 0 待填）
| 场景 | fps | p50ms | p95ms | cpu% | privMB | dda/s | own_us |
|---|---|---|---|---|---|---|---|
| single-drag | | | | | | | |
| many-drag | | | | | | | |
| many-drag lat=2+SIM | | | | | | | |


---

## 验证记录（2026-10-20，重构主体完成后）

### 功能 / 单元 / 集成
- `cargo test --release --workspace`：**147 passed / 0 failed**
  （lib 单元 129 + `tests/api_surface.rs` 集成 11 + playground 5 + doctest 2；重构前 25）
- `cargo clippy --workspace --all-targets --release`：**0 warning**
- `cargo fmt --all --check`：通过
- `node scripts/check.mjs` / `cargo xtask check`：同上三件套

### 像素级视觉取证（tools/cmp_visual.py，新口径）
- 口径改为**稳态帧**（`GLASS_INTERNAL_SHOT_AT_MS=2500`）+ 光标物理停放（`SetCursorPos` + 看门狗）
  + 停放点离玻璃 > 3*mouse_radius（鼠标点光有限支撑窗外，gate 精确为 0）。
- `single` 跨版本（重构前 exe vs 重构后）：**deep/interior/ring/outside 四段全 0 差异 = 逐位一致** ✅
- `many` 同一构建两次：**全 0**（确定性）✅
- `many` 跨版本：差异 100% 归因到圆角半径的**有意变更**
  （圆角邻域 r+6 内 36729 px / 最大 136；邻域外 3105 px / 最大 2 = 阴影尾部残差，阈值内）✅
  原因：旧主循环 `if target_id >= 0 { 只改选中块 } else { 改全部 }`，而 selected 恒为最后一块，
  导致 20 块里 19 块的 radius 永远停在 `Block::new` 的硬编码 28；新库统一由样式派生（需求 F1）。
- 光标在深内部两点移动：**deep 段 0 差异**（背景不颤抖）✅

### 其余回归阀门
| 脚本 | 结果 |
|---|---|
| verify_dirty ×3 | PASS ×3 |
| verify_mouse_isolation | PASS |
| verify_validity | PASS |
| verify_crossscreen | PASS |
| verify_env_ema | PASS ×3（补 `GLASS_ADAPTIVE=0` 后；见下） |
| verify_adaptive | **环境阻塞**（见下） |
| gate_perf | **环境阻塞**（见下） |

### 取证脚本本身的三处缺陷（本轮修复，均已在脚本内注释说明）
1. `GLASS_CURSOR_AT` 过去只设初值、主循环立刻回到真实光标 → 「光标移动」用例恒为 0 差异（空转测试）。
   重构后该变量**全程钉死**（`App::pinned_cursor`），cmp_visual 另加物理停放。
2. 像素等价脚本未关 `GLASS_ADAPTIVE`：降档会换 frost 分辨率 / 关焦散色散，同一份代码两次跑都会对不上。
   已给 verify_* 补 `GLASS_ADAPTIVE=0`；**verify_adaptive 除外**（它验证的就是闭环，误关过一次已撤回）。
3. 库默认不再自建窗口；取证脚本显式 `GLASS_DEFAULT_BLOCK=1`。

### 性能：当前机器不在健康测量窗口（环境阻塞，非回归）
- 项目规则：性能结论只在 `rel_max<5ms`、`acq_max<25ms` 的窗口下给出。
- 实测当前窗口 `rel_max ≈ 200~257ms`（DDA `ReleaseFrame` 阻塞），**旧二进制同条件同样如此**：
  背靠背 A/B（single-drag，同环境同参数）：
  OLD `fps=6.3 acq=0.322 rel=108.2 rel_max=225.6` / NEW `fps=8.4 acq=0.277 rel=92.1 rel_max=199.9`
  → 两者统计上无差别，NEW 的 `own_us`/stage 分段（A/B/P/C）与 OLD 同噪声水平。
- 会话早期健康窗口的基线（旧二进制）：single-drag 56.5fps / p50 10.80ms / own_us 61µs；
  many-drag 51.6fps / own_us 61µs；lat=2+合成负载 85.3fps / own_us 70µs；Private 159.8~161.2 MB。
- **待办：在健康窗口复跑 `python tools/gate_perf.py` 与 `verify_adaptive.py` 并回填本表。**


---

## 自检轮（2026-10-21，独立 check agent 对抗性复核 + 主会话修复）

独立复核（只读静态审查）报 13 条，复核后确认 11 条成立，已全部修复并重跑阀门：

| 编号 | 问题 | 修复 |
|---|---|---|
| P0-1 | API 改 z 序不脏化合成器 → `bring_to_front()` 端到端无效 | 新增 `Compositor::invalidate_present()`（只作废 present 账本），`SetZ` 变化时调用 |
| P1-1a | 采集门控读陈旧 `b.style` → `set_default_style(transparent)` 后采集不降频 | 新增单一来源 `Block::effective_style(default)`，渲染/快照/采集门控三处统一走它 |
| P1-1b | `SetDefaultStyle` 不脏化 → 静止桌面拖滑杆不生效 | 补 invalidate（mode 翻转用全量，其余用 `invalidate_present`） |
| P1-2 | border 原点 clamp 与玻璃 pass 不同源 → 块半出屏时白框错位 | `draw_borders` 的 b0 改走 `DrawBlock::draw_rect()`（未 clamp），clamp 只给 scissor |
| P1-3 | 采集运行期重建丢 `capture.backend` 偏好 | 改 `for_monitor_pref(.., backend_pref)`；删零调用点的 `for_monitor` |
| P1-4 | `interactive=false` 仍摘 `WS_EX_TRANSPARENT` 吃点击 | `DrawBlock.interactive` + 穿透判定排除（窗口区域仍包含） |
| P1-5 | `QualityCtrl::disable()` 误清 `fps_cap` | `disable()` 只清自适应间隔；补单测 `disable_keeps_the_fps_cap` |
| P2-1 | `EngineConfig::adaptive_quality` 无读取点 | 接线（与 `no_adaptive`/`GLASS_ADAPTIVE=0` 任一关闭即禁用） |
| P2-2 | `AlwaysOnBottom → Normal` 不恢复 Z 序 | 删除 `level::apply` 的「样式没变就跳过」早退 |
| P2-3 | 全透明屏谎报 `background_valid`（不变量 2） | 退化早退分支补 `needs_backdrop` 保护 |
| P2-4 | 读-改-写 setter 同帧互相覆盖 | 新增 `Command::PatchStyle`/`SetResizable`，渲染线程权威状态上叠加；补单测 |
| P2-5 | 退出后命令队列无限增长 / `start()` 可双开 | `Shared.stopped/started` 位；setter no-op、重复 start 返回 `Err(Shutdown)` |
| P2-7 | `GLASS_SIM_BACKDROP` 脏区量纲错（绝对坐标当局部坐标） | 按屏换算到画布局部坐标 |
| P2-8 | 透明块以 `mouse_radius=20` 参与影响盒 → 光标白白驱动整帧 | 按 `mouse_intensity > 0` 门控（`max_grip` 不门控） |
| F3.1 补强 | 透明屏 4Hz 保活采集在 DDA 病态机器上 ReleaseFrame 阻塞到 16s | 全透明/空屏**完全停采**；液态块落回时 `just_activated` 强制取新帧 |

过度设计清理：`capture_generation` 字段、`draw_blocks` 的 `dirty` 死参数与死分支、
`ErrorKind::NotSupported`、`edges_active/corners_active` 恢复为唯一判据、`pass_a_now` 降私有、
`BorderTarget` 降 `pub(crate)`、`.gitignore` 的 tauri 遗留、`RAW_TARGETS` 去掉两个预留靶标、
`hit_resize` 改零分配单遍扫描、`render_monitor` 去掉每帧 String clone。

新增/调整测试：`disable_keeps_the_fps_cap`、`fps_cap_and_adaptive_level_take_the_stricter_one`、
`effective_style_follows_the_engine_default_while_inheriting`、
`style_patches_enqueue_in_order_and_never_read_the_snapshot`、
`resizable_toggle_only_flips_the_enabled_bit`、handle 未知窗口语义改写。
**测试总数 147 → 151。**

阀门复跑（修复后）：cmp_visual single 四段全 0 / many 归因不变 / cursor_deep=0；
verify_dirty ×3、verify_mouse_isolation、verify_validity、verify_env_ema 全 PASS；
`node scripts/check.mjs` 全绿。

性能阀门新增第四场景 `transparent-drag`（`perf_probe --case transparent-drag`，
playground 以 `--transparent --no-demo-pair` 启动）；硬门槛与液态单块同一套，
另加 WARN：透明 `own_us` 高于液态单块 +10µs 即提示跳过链路可能断了。
实测（不健康窗口，仅供参考）：透明空闲 cpu 0.39%、Private 121.6 MB（液态 157 MB）、
`pass_a/frame = 0.00`、采集段 cap_max 从 15989 ms 降到 0.001 ms。

## 优化轮 2（border 加粗 / movable / 发布脚本）取证记录

- **border「看着细」根因**：`border_ps.hlsl` 的 band 原是 `1 - smoothstep(0, bw, inside)`，
  从第 1 像素就渐隐；像素剖面实测 bw=5 时 t=2 处 alpha=0.49、t=4 归零，人眼只读得到约 2 px。
  改为实芯 + 外沿 AA（`aaBand = min(aa, bw*0.5)`、`1 - smoothstep(bw-aaBand, bw, ·)`）后：
  t=1..3 纯白 255、t=4 单个 AA 过渡像素；面板下发 borderWidth=20 实测 19 px 实芯 + 1 px AA。
  取证图：`tools/fb3_edge_right_zoom.png` / `fb3_corner_br_zoom.png`（4x 近邻，`tools/shot_zoom.py`）。
- **面板冒烟**：`tools/smoke_panel.py` 全过（movable 双向、feedback 默认 5/1、applyToAll 链路）。
- **verify_dirty 间歇残差**：新二进制 6 次里 2 次出现 0.03% / 最大通道差 4 的单行残差
  （位置在金字塔刷新区接缝、非 border 支撑窗内）；旧二进制 1/1 干净；随后新二进制连跑 4 次全 0。
  归类为「桌面繁忙窗口的间歇残差」（BASELINE.md 既知类），健康窗口复跑已全绿；
  两次 FAIL 均发生在与其它取证/构建并发之后。
- **verify_mouse_isolation**：PASS（hover/拖拽影响仍完全受控）。
