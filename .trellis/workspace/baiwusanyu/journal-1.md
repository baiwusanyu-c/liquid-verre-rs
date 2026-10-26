# Journal - baiwusanyu (Part 1)

> AI development session journal
> Started: 2026-10-20

---



## Session 1: 视觉缺陷根治 + 性能阀门 + Trellis spec 落地（重构前基线）
<!-- trellis-session: v=2 fp=00690d86c5544293 -->

**Date**: 2026-10-20
**Task**: 视觉缺陷根治 + 性能阀门 + Trellis spec 落地（重构前基线）
**Branch**: `master`

### Summary

第九~十四轮：跨块串扰(3σ gate)、控制器误降档(own-work 判据)、切档内存、HDR 过曝(自动切 WGC)、拖一块其他块闪动(Clear 忽略 scissor + flip 不持久 -> 每帧自包含)、金字塔幂等外扩、EMA 门控、debug 溢出 panic、latency 1/2 取舍(默认 1)；25 单测 + gate_perf + 8 回归脚本成为重构阀门；Trellis spec 8 份落地 + CLAUDE.md/AGENTS.md/README 更新；基线 tag baseline/pre-refactor、分支 backup/pre-refactor-baseline、重构起点 4dca041

### Git Commits

| Hash | Message |
|------|---------|
| `2be3393` | fix: 跨块串扰/漏更新/HDR 过曝三组视觉缺陷修复与取证 |
| `f58389d` | fix: 根治「拖一块、其他块闪动」——画布每帧自包含 + 金字塔幂等 + 启动 panic |
| `d7585a9` | perf: 20 块拖拽达到 >=90fps —— GLASS_FRAME_LATENCY 默认 2 + 下采样链幂等补完 |
| `bcf9511` | merge+revert: 合并分叉 tip；GLASS_FRAME_LATENCY 默认回退 1（跟手优先） |
| `82fc311` | chore: 重构前基线快照 BASELINE.md（指标/回归清单/关键不变量/已知偏离） |
| `4dca041` | test: 重构前质量阀门 —— 25 个单元测试 + gate_perf.py + BASELINE 阀门章节 |
| `6db6345` | docs/spec: Trellis 规范落地 + 重构基线文档更新 + 多平台集成入库 |

### Status

[OK] **Completed**


## Session 2: liquid-verre-rs 库化重构（workspace + 双模式 + 8 方向缩放 + 层级 + 像素级验收）
<!-- trellis-session: v=2 fp=83c61d60a62757da -->

**Date**: 2026-10-20
**Task**: liquid-verre-rs 库化重构（workspace + 双模式 + 8 方向缩放 + 层级 + 像素级验收）
**Branch**: `codex/liquid-verre-rs-refactor`

### Summary

单 bin crate 重构为 workspace 库项目；147 测试全绿、clippy 0 警告；single 跨版本逐位一致；性能阀门待健康窗口复测；文件夹重命名待目录句柄释放。

### Main Changes

## 做了什么

把 `liquid-glass`（单 bin crate）重构成 `liquid-verre-rs`（Cargo workspace 库项目），
并落地全部功能需求。未 commit（项目约定：未经明确要求不提交）。

### 结构
- workspace：`crates/liquid-verre`（库，lib 名 `liquid_verre`）+ `playground`（dev 控制面板，
  原 `src/ui/`，非业务代码）+ `xtask`（build/check/package/gate/clean）。
- 库内分层：`api/`（唯一公共面）/ `core/`（纯逻辑，零 Win32）/ `runtime/`（主循环）/
  `platform/`（Win32 集中地）/ `gfx/` / `capture/` / `input/` / `diag/`。
- `main.rs`（2133 行 god object）拆为 runtime/{mod,app,monitor,commands}.rs + platform/*。
- 构建脚本从 PowerShell 迁到 **Node**（`scripts/*.mjs`，零 npm 依赖，含自实现 zip）；
  `cargo xtask package` 委托 `node scripts/zip.mjs`；target triple 改读 `rustc -vV`。
- 删除死文件：`src/gfx/w_mode_compositor.rs`（孤儿）、`shaders/reduce_env.hlsl`。

### 功能
- F1 全参数配置化：`LiquidParams` 全字段 + serde；playground 面板按 `schema.rs` 动态生成控件，
  并有单测保证「库支持的每个参数都出现在面板上」。
- F2 8 方向缩放（`core/handle.rs` 先角后边）+ 锚定不动边；border 反馈改为**独立叠加 pass**
  （`border_vs/ps.hlsl` + `BorderCb` + `draw_borders`），直边到切点为止、对角沿弧；
  `ResizeOptions.edges/corners/enabled` 可分别禁用。
- F3 双模式：`GlassMode::Liquid/Transparent`；透明模式只 3 个参数、着色器早退不采样纹理、
  全透明屏整条 Pass A 跳过 + 采集降保活频率。
- F4 层级：`WindowLevel::{AlwaysOnTop,Normal,ClickToFront,AlwaysOnBottom}`（全程 SWP_NOACTIVATE
  不抢焦点）+ 块 z 序 `ZOrder`/bring_to_front/send_to_back（归一化防 i32 溢出）。
- E2 错误/日志：`Error`/`ErrorKind`(10 类)/`Result`；`log` facade + 默认 stderr logger
  严格保留 `[boot]/[info]/[warn]/[verify]/[metrics]/[adaptive]` 前缀（tools 解析依赖）。

### 取证抓出并修掉的真实缺陷
1. border 的 premultiplied source-over 衰减因子写错（用 `1-alpha` 而非 `1-fb`）→ border 处变暗 120 级。
2. `wait_for_captures` 漏调 `set_capture` → 启动白等满 6s budget。
3. 旧面板 isize 量化篡改默认值（tint_opacity 0.018→0.01、blur_min→2.4），导致旧二进制
   引导帧与稳态帧本身不一致（873748 px / max 140）；库默认值对齐「用户实际看到的稳态值」，
   且 `blur_min` 必须写表达式 `20.0 * 0.12`（字面量 2.4 差 1 ULP ≈ 2400 个 ±1 LSB 像素）。
4. 旧主循环圆角滑杆只写「选中块」→ 20 块里 19 块 radius 永停 28；新库统一由样式派生（有意变更，已量化归因）。
5. `GLASS_CURSOR_AT` 过去只设初值 → 「光标移动」取证用例空转；现全程钉死 + cmp_visual 物理停放光标。
6. 像素等价脚本未关 `GLASS_ADAPTIVE`（verify_adaptive 除外，它验证的就是闭环）。
7. compositor 里手写的影响盒常量与 `cursor_influence_px` 漂移 → 收敛到 `core::geometry` 单一来源。

## 验证
- `cargo test --release --workspace`：**147 passed**（129 单元 + 11 集成 + 5 playground + 2 doctest；重构前 25）。
- `cargo clippy --workspace --all-targets --release -- -D warnings`：0 warning；`cargo fmt --check` 通过；
  `node scripts/check.mjs` 全绿。
- `cmp_visual.py`（新口径：稳态帧 + 光标物理停放 + 四段 oracle）：**single 跨版本四段全 0 = 逐位一致**；
  many 同构建两次全 0；many 跨版本差异 100% 归因到圆角有意变更（邻域外 max=2 <= 阈值）；
  光标深内部移动 deep 段 0。
- verify_dirty x3 / verify_mouse_isolation / verify_validity / verify_crossscreen / verify_env_ema x3：PASS。
- playground HTTP 端到端冒烟：模式切换 / 逐窗口部分更新 / 嵌套 resize 配置 / 层级 / 增删 / 事件通道 全通过。
- `cargo xtask package`：dist/ 产物 + build-info.json + Node zip（13 文件）验证可解包。
- **环境阻塞**：gate_perf 与 verify_adaptive 当前机器 `rel_max≈200~257ms`（DDA ReleaseFrame 阻塞），
  远超健康窗口 `rel_max<5ms`；旧二进制同条件同样 FAIL，背靠背 A/B 无差别 → 非回归。
  待健康窗口复测并回填 BASELINE.md / README 性能表（占位已留）。

## 未完成 / 下一步
1. **文件夹重命名**：`node scripts/rename-workspace.mjs`（Codex 集成终端持有仓库根句柄导致 EBUSY；
   在该终端 `cd ..` 或关掉标签后重试，或 `--retry 120`）。重命名后需在新目录重跑 check + cmp_visual。
2. 健康窗口复跑 `python tools/gate_perf.py` 与 `verify_adaptive.py`，回填性能表。
3. README 里 13 处 `<!-- TODO(verify) -->` 占位（性能复测数字 + 最低 Windows 版本）。
4. 用户明确要求后再 commit（分支 `codex/liquid-verre-rs-refactor`）。


### Git Commits

(No commits - planning session)

### Status

[OK] **Completed**

### Next Steps

- 重命名文件夹 + 健康窗口复跑 gate_perf/verify_adaptive + 回填性能表


## Session 3: 自检轮：独立审查 15 缺陷修复 + 过度设计清理 + 取证 oracle 修正
<!-- trellis-session: v=2 fp=f88845f0c49cb182 -->

**Date**: 2026-10-21
**Task**: 自检轮：独立审查 15 缺陷修复 + 过度设计清理 + 取证 oracle 修正
**Branch**: `codex/liquid-verre-rs-refactor`

### Summary

对照需求/过度设计/逻辑bug 三维度自检；修复 15 条（含 z 序不重画、border 原点、interactive 穿透、fps_cap 被 disable 清掉等）；测试 147→151；cmp_visual 与 7 个像素阀门全绿；重命名与性能复跑待环境。

### Main Changes

## 做了什么（自检轮，2026-10-21）

按用户要求做三项严格自检：① 对照原始需求逐条验收；② 过度设计审查；③ 逻辑 bug 审查。
方式：派独立只读 check agent（trellis-check）对抗性复核 + 主会话逐条复核修复。

### 修复的缺陷（15 条，均有独立审查编号）
P0-1 API 改 z 序不重画（新增 Compositor::invalidate_present）；P1-1a 采集门控读陈旧 style
（新增 Block::effective_style 单一来源）；P1-1b SetDefaultStyle 不脏化；P1-2 border 原点与
玻璃 pass 不同源（块半出屏错位）；P1-3 采集重建丢 backend 偏好；P1-4 interactive=false 仍吃点击；
P1-5 disable() 误清 fps_cap；P2-1 adaptive_quality 死字段接线；P2-2 AlwaysOnBottom→Normal 不恢复；
P2-3 全透明屏谎报 background_valid（不变量 2）；P2-4 读-改-写 setter 同帧互覆盖（改细粒度命令
PatchStyle/SetResizable）；P2-5 退出后队列无限增长 / start 可双开；P2-7 SIM_BACKDROP 量纲错；
P2-8 透明块以 mouse_radius 参与影响盒；F3.1 补强：全透明/空屏完全停采（消掉 16s 级 ReleaseFrame 停顿）。

### 过度设计清理
删 capture_generation、draw_blocks 的 dirty 死参数与死分支、ErrorKind::NotSupported、
.gitignore 的 tauri 遗留、RAW_TARGETS 两个预留靶标；hit_resize 改零分配；render_monitor 去每帧
String clone；pass_a_now 降私有；BorderTarget 降 pub(crate)；edges_active/corners_active 恢复为唯一判据。
保留并写明理由：StyleCache / FrameInput / SpawnSpec / ResizeHandle::public / Snapshot 双缓冲（注释频率已纠正）。

### 取证框架修正
cmp_visual：冻掉环境色 EMA 的墙钟 dt 依赖（GLASS_ENV_EMA=0）；many 的 now-vs-now 判据改 ≤2 LSB 容差
（部分刷新的既有时序敏感，verify_dirty 在同一次运行内严格证明增量==全量）；single 仍硬 0。
gate_perf：新增第四场景 transparent-drag（硬门槛同液态单块 + own_us 劣化 WARN）。

### 验证
check.mjs 全绿（fmt + clippy -D warnings + test）；测试 147 → **151**。
cmp_visual PASS（single 四段全 0 = 跨版本逐位一致；many 归因圆角有意变更；cursor_deep=0）。
verify_dirty ×3 / mouse_isolation / validity / crossscreen / env_ema 全 PASS。
transparent 实测（不健康窗口）：空闲 cpu 0.39%、Private 121.6MB（液态 157MB）、pass_a/frame=0、
采集段 cap_max 15989ms → 0.001ms。
性能 A/B（3×3 轮转，昨日）：fps 中位 88.3(新) vs 86.9(旧)、prep 26 vs 24µs → 持平。

### 仍未完成
1. 文件夹重命名（Codex 集成终端持有根目录句柄，EBUSY）：`node scripts/rename-workspace.mjs --retry 120`。
2. gate_perf / verify_adaptive 待健康窗口（rel_max<5ms）复跑回填；今日 rel_max 18~37ms，OLD/NEW 同条件同病。
3. README 13 处 TODO(verify) 占位（性能复测数字 + 最低 Windows 版本）。


### Git Commits

(No commits - planning session)

### Status

[OK] **Completed**

### Next Steps

- 重命名文件夹 + 健康窗口复跑 gate_perf/verify_adaptive + 回填 README 性能表


## Session 4: 优化轮 2 收尾：border 实芯修复取证 + 全阀门绿 + 提交归档
<!-- trellis-session: v=2 fp=058977ac0b7df62f -->

**Date**: 2026-10-21
**Task**: 优化轮 2 收尾：border 实芯修复取证 + 全阀门绿 + 提交归档
**Branch**: `codex/liquid-verre-rs-refactor`

### Summary

border 加粗根因定位（band 渐隐改实芯+AA，像素剖面与 4x 取证图证明厚度受控）；movable 面板冒烟工具化（tools/smoke_panel.py）；取证放大工具 tools/shot_zoom.py；gate_perf/verify_adaptive/verify_dirty(x4)/cmp_visual/smoke_panel 全绿；文档计数与 border 公式同步（README/CHANGELOG/spec/performance_report）；清理 4.6GB 取证中间产物；单次提交 64768b1 后归档任务。

### Git Commits

| Hash | Message |
|------|---------|
| `64768b1` | refactor!: 库化重构为 liquid-verre-rs workspace（0.4.1 优化轮合入） |

### Status

[OK] **Completed**


## Session 5: 遮挡优化 A+B1：隐藏屏停产与逐块摘触发源（含 Progman 误判修复、性能阀门两档化）
<!-- trellis-session: v=2 fp=767fd6ad6f71298d -->

**Date**: 2026-10-23
**Task**: 遮挡优化 A+B1：隐藏屏停产与逐块摘触发源（含 Progman 误判修复、性能阀门两档化）
**Branch**: `master`

### Summary

落地遮挡优化 A（hidden_fs 升级为停产门）+ B1（被完整遮挡的块不再驱动采集与光标触发源，不改绘制集合），并修掉身份日志当场抓到的 Progman 误判；性能阀门改成两档判定，重标了压在量化悬崖上的 many-drag 门槛。全程取证通过。

### Main Changes

- A：hidden_fs 从「可见性位」升级为「停产门」—— 采集段与渲染段整屏跳过，恢复走 invalidate() + ready=false 的既有采集失效路径，零陈旧帧
- B1：platform/occlusion.rs 的 8 Hz z 序扫描 + core 的 rect_fully_covered / occlusion_step —— 被不透明窗口完整覆盖绘制矩形的块不再计入 wants_bd 与 mouse_relevant，不改绘制集合
- 顺带修：屏从停采恢复那一帧现在会 compositor.invalidate()（遮挡移开与全屏退出都不经过 Command，原「靠 commands 层兜底」已不成立）
- fullscreen_app_on：命中日志打出触发窗口 class/title/rect（新增 fullscreen_app_on 返回 Option<HWND>）+ cloaked 否决 + 排除 Progman 桌面本身 + 新增 EngineConfig.fullscreen_hide
- gate_perf 改成两档判定（有 VERRE_OLD_EXE 走同机 A/B 相对不劣化，否则退回绝对门槛）+ spread_pct 噪声可辨识度 + REPS 默认 3 + GATE_SECONDS
- 重标 many-drag(lat=1)：fps>=60 改为 fps>=55 且 p50<=17.5ms（p50 恒为 2 个 vsync、0.4.1 参考臂同一机器只有 59.3），p50 那条是防回归主力
- 新增 tools/verify_occlusion.py（八阶段）与 tools/test_gate_logic.py（21 条纯逻辑用例）；12 个取证脚本补 --no-browser

### Git Commits

| Hash | Message |
|------|---------|
| `6e88cfb` | pref: 遮挡不绘制优化 |
| `7219a6c` | pref: 遮挡不绘制优化 |
| `31aea6f` | fix(tools): 取证脚本补 --no-browser；gate_perf 两档判定并重标 many-drag 门槛 |

### Testing

- [OK] cargo test --release --workspace 197 passed；clippy -D warnings / fmt --check / node scripts/check.mjs 全绿
- [OK] verify_occlusion.py 8/8 PASS，--off 5/5 PASS；AC2/AC6 像素取证差异像素均为 0
- [OK] BASELINE 八项回归（verify_dirty x3、verify_validity 虚报 0/漏画 0、verify_mouse_isolation、verify_crossscreen、repro_tremble far/many、verify_adaptive、ab_latency、ab_memory）全 PASS
- [OK] 遮挡场景进程 CPU 11.36% -> 0.95%（-92%，约 12 倍）；B1 单独 1.47% -> 0.95%（-35%），两轮重复同向
- [OK] gate_perf 相对 A/B 与绝对档在干净机器上均 PASS（退出码 0）：116.0 / 61.4 / 105.2 / 93.5 fps，own_us 14~36 us

### Status

[OK] **Completed**

### Next Steps

- B2（真正不绘制被遮挡的块）留待独立任务：前置是 gfx/compositor.rs 的 prev_rects.len() != cur_rects.len() 整屏重画判据先改成按 id 差集，并给消失的块补擦旧矩形
- fullscreen_app_on 再误报时先看新日志里的窗口身份；不要采用「该窗口必须在画布之上」这条判据（真游戏同样在 TOPMOST 画布之下，会把 R-16 废掉）
- 可选：SetWinEventHook 做亚帧恢复，替代当前 8 Hz 轮询带来的 ≤125 ms 陈旧桌面窗口
- 一次非复现的 prep 单圈 3061 ms stall 归因未确认，已变成 verify_occlusion 的常驻断言（prep 中位 > 20 ms 即 FAIL），复发可自动抓到


## Session 6: 库窗口即本体能力补齐：P0-1 内容层 + P0-2 输入路由 + P1 焦点路由(FocusHint)
<!-- trellis-session: v=2 fp=cb341e44e648640e -->

**Date**: 2026-10-26
**Task**: 库窗口即本体能力补齐：P0-1 内容层 + P0-2 输入路由 + P1 焦点路由(FocusHint)
**Branch**: `master`

### Summary

Trellis 任务树 10-23-glass-window-host-capabilities 全部完成并归档（move-filter 保留 planning 待启）。交付：GlassWindow 新增 update_content/clear_content（逐块交错绘制、dirty 子区上传、无内容零开销）、GlassInputSink 输入路由（六规则状态机 core/routing.rs 31 单测）、set_focus_hwnd 事件化（FocusHint，库零焦点 FFI）。行为取证三件套 verify_input_sink/verify_content/verify_focus_drop 均带判别力自检与防假门禁设计。过程战果：抓到并修复 6 个真缺陷（缩放会话收尾绑错分支、wheel 高位字、UpdateSubresource 子矩形源指针、clear 不脏化、region_dirty 误置、attach 跨线程挂死渲染线程——最后一枚经插桩二分实锤后裁决库侧焦点全部事件化）。第一门禁全程守住：无内容/无 sink 构建 vs 0.4.2 cmp_visual single 四段全 0，gate_perf 同机 A/B 全部 -1.1%~0.6% 无回归，8 回归脚本 + verify_dirty x3 多轮全绿。架构再审（research/architecture-native-window.md）：用户拍板维持画布+能力补齐路线，否决 per-window 重写。

### Git Commits

| Hash | Message |
|------|---------|
| `d4ea46b` | feat(host): P0-1 内容层 + P0-2 输入路由 + P1 焦点路由（FocusHint）+ 三行为取证脚本，全量门禁绿 |

### Status

[OK] **Completed**
