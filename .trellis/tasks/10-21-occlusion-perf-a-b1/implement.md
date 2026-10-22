# 执行计划：A 隐藏屏停产 + B1 被遮挡块不进触发源

> 分 5 个阶段，每阶段都有独立验证点。**阶段 2 与阶段 3 可并行**（写集不重叠）；
> 阶段 1 与阶段 4 都改 `runtime/mod.rs`，必须由**同一个人**顺序做（避免冲突）。

## 全局：中文写入的编码配方（已实测踩坑，务必遵守）

PowerShell 管道用 UTF-8，但 Python 的 stdin 默认按 cp936 解码，所以「here-string 管道给
`python -`」这种写法直接写会得到乱码（主会话写 `prd.md` 时实测中招，且**不可逆**：
用 `s.encode('cp936').decode('utf-8')` 还原会抛 `UnicodeEncodeError`）。

每次写含中文的文件，必须在**同一条命令**里先设两个编码开关，再用 here-string 把内容
通过管道交给 python 落盘：

- `$env:PYTHONUTF8="1"`（让 Python 的 stdin/stdout 走 UTF-8）
- `$OutputEncoding=[System.Text.UTF8Encoding]::new($false)`（让 PowerShell 管道写 UTF-8 无 BOM）
- 落盘：`python -c "import sys,io; io.open(r'<路径>','w',encoding='utf-8',newline='\n').write(sys.stdin.read())"`

自检一律用「Python 读回来 + 只打印 ASCII 判定」，因为 `chcp` 是 936，终端**回显**中文常常
显示成乱码，那**不代表文件坏了**：

- `python -c "import io;s=io.open(r'<路径>',encoding='utf-8').read();print('OK' if '<中文短语>' in s else 'BAD')"`

仓库现有文件是 **LF + 无 BOM**，`newline='\n'` 不要漏。

⚠ 另一个坑：here-string 的结束标记是「行首的单引号 + at 符号」。如果你要在**文件内容里**
写这个配方本身的示例，不要原样写结束标记，否则外层 here-string 会被提前终止
（主会话写本文件时实测踩过：ParserError `Missing type name after '['`）。

## 阶段 0：前置阅读（不许跳过）

- [ ] `.trellis/spec/backend/rendering-pipeline.md`（9 条不变量、逐帧管线契约）
- [ ] `.trellis/spec/backend/directory-structure.md`（分层边界、`unsafe` 分布）
- [ ] `.trellis/spec/backend/logging-guidelines.md`（前缀即接口）
- [ ] `.trellis/spec/backend/quality-guidelines.md`（禁止/必须模式）
- [ ] `.trellis/spec/guides/graphics-change-checklist.md`（改管线前的 11 问）
- [ ] 本任务 `prd.md` + `design.md`

## 阶段 1：A —— `hidden_fs` 停产门

**文件（独占）**：`crates/liquid-verre/src/runtime/mod.rs`

- [ ] 1.1 采集段：`copied_px = 0` 之后、`has_blocks` 之前插入
      `if mon.hidden_fs { mon.idle_capturing = true; continue; }`（design 1.1；
      中文注释写「为什么」，并引 R-A2 的忙等前科：DDA ACCESS_LOST 时 96 kHz / 83% CPU）
- [ ] 1.2 渲染段（约 `runtime/mod.rs:820`）：在 `!ready && needs_backdrop` 跳过条件**之前**加 `hidden_fs` 跳过
- [ ] 1.3 恢复边沿（约 `runtime/mod.rs:717-735` 的 `if hit != mon.hidden_fs`）：
      `!hit` 时 `mon.compositor.invalidate()`，且 `if mon.needs_backdrop { mon.ready = false; }`，
      再 `window::apply_visibility(mon)`（design 1.3，`needs_backdrop` 条件**不可省**，
      否则全透明屏会被永久隐藏）
- [ ] 1.4 **验证**：`cargo build --release --workspace` + 手工跑
      `GLASS_DEFAULT_BLOCK=1 GLASS_METRICS=1 target/release/liquid-verre-playground.exe`，
      用一个无边框铺满屏的前台窗口压住，确认 `[metrics]` 的 `dda_acq` 掉到 `0.0/s`、
      `fps` 掉到 0，窗口移开后玻璃正常回来且**不闪旧桌面**
- [ ] 1.5 既有单测不回归：`cargo test --release -p liquid-verre-rs`
      （特别是 `runtime/mod.rs` 末尾引用 `IDLE_WAIT_MS` 的速率下限单测）

**回滚点**：阶段 1 改动集中在 `runtime/mod.rs` 三处。

## 阶段 2：纯逻辑层（core，零 Win32，可单测）—— 与阶段 3 并行

**文件（独占）**：`core/geometry.rs`、`core/block.rs`、`core/draw.rs`
（+ `runtime/app.rs` 里 `DrawBlock` 字面量的**一行** `occluded: b.occluded,`）

- [ ] 2.1 `core/geometry.rs`：`pub fn rect_fully_covered(target: [i32;4], covers: &[[i32;4]]) -> bool`
      （分割法差集；半开区间；零面积 target 返回 true；相邻边接触算覆盖；空 covers 返回 false）
- [ ] 2.2 `core/geometry.rs` 单测（至少 8 例）：单矩形全覆盖 / 两矩形左右拼接 / 上下拼接 /
      L 形三矩形 / 只覆盖一半为 false / 空 covers 为 false / 零面积 target 为 true /
      相邻边恰好接触为 true / cover 比 target 大为 true / target 含负坐标
- [ ] 2.3 `core/block.rs`：`OCCLUSION_CONFIRM_POLLS: i8 = 2` +
      `pub(crate) fn occlusion_step(streak: i8, hit: bool) -> (bool, i8)`
- [ ] 2.4 `Block` 增 `pub occluded: bool` / `pub occl_streak: i8`，`Block::new` 初始化，
      **同步所有构造点**（`grep -rn "Block {" crates/ playground/`）
- [ ] 2.5 `occlusion_step` 单测：首次命中不置位 / 连续 2 次置位 / 置位后继续命中保持且
      streak 饱和不溢出 / 一次未命中立即解除且 streak 归零
- [ ] 2.6 `core/draw.rs`：`DrawBlock` 增 `pub occluded: bool`，同步单测构造函数 `mk()`
      与 `runtime/app.rs` 的字面量
- [ ] 2.7 **验证**：`cargo test --release -p liquid-verre-rs core::` +
      `cargo clippy -p liquid-verre-rs --all-targets --release -- -D warnings` +
      `cargo build --release --workspace`
- [ ] 2.8 **不变量检查**：`grep -rn "unsafe" crates/liquid-verre/src/core` 与
      `grep -rn "windows::" crates/liquid-verre/src/core` 都必须为空（R-D2 / R-D3）

## 阶段 3：Win32 扫描层（platform，unsafe 集中处）—— 与阶段 2 并行

**文件（独占）**：`platform/occlusion.rs`（新增）、`platform/mod.rs`、`crates/liquid-verre/Cargo.toml`

- [ ] 3.1 `platform/mod.rs` 加 `pub(crate) mod occlusion;`
- [ ] 3.2 `pub(crate) fn is_opaque_occluder(exstyle: u32, visible: bool, iconic: bool, has_rgn: bool, cloaked: bool) -> bool`
      —— 纯逻辑，按 design 2.5（`WS_EX_LAYERED` / `WS_EX_TRANSPARENT` 返回 false）
- [ ] 3.3 `is_opaque_occluder` 单测（至少 6 例：全通过 + 每个否决条件各一例）
- [ ] 3.4 `pub(crate) fn opaque_rects_above(stop_at: HWND, bounds: [i32;4], out: &mut Vec<[i32;4]>) -> usize`
      —— `EnumWindows` 回调，**严格按 design 2.5 的 7 步顺序**（便宜的在前、`DwmGetWindowAttribute` 最后）
- [ ] 3.5 `GetWindowRgn` 的 `HRGN` **必须** `DeleteObject`（R-D4，静默泄漏，8 Hz 下几分钟就炸）
- [ ] 3.6 回调**不得 panic**（穿越 FFI 是 UB）：无索引越界 / 无 `unwrap` / 无整数溢出，
      或 `catch_unwind` + `AssertUnwindSafe`
- [ ] 3.7 确认 API 在 `windows = "0.58"` 可用：`EnumWindows`、`IsWindowVisible`、`IsIconic`、
      `GetWindowRect`、`GetWindowLongPtrW`、`GWL_EXSTYLE`、`WS_EX_LAYERED`、`WS_EX_TRANSPARENT`、
      `GetWindowRgn`、`DeleteObject`、`DwmGetWindowAttribute` + `DWMWA_CLOAKED`。
      常量未导出就本地 `const` 定义并注明（前科：`MONITORINFOF_PRIMARY`）
- [ ] 3.8 按需补 `Cargo.toml` 的 windows features（先读现有列表，缺什么补什么，不要重写）
- [ ] 3.9 **验证**：`cargo build --release -p liquid-verre-rs` + `cargo test --release -p liquid-verre-rs platform::`
      + `cargo fmt --all -- --check`。
      **预期现象**：消费方（阶段 4 的 `runtime/occlusion.rs`）还不存在，clippy 会报这两个
      `pub(crate)` 函数 `dead_code`。**不要**加 `#[allow(dead_code)]`（会残留成技术债），
      也不要为了让 clippy 绿去改 `runtime/`；把警告原样记进交付报告即可。

## 阶段 4：编排 + 消费点 + API 面

**文件**：`runtime/occlusion.rs`（新增）、`runtime/mod.rs`、`runtime/app.rs`、
`api/config.rs`、`api/handle.rs`、`diag/env.rs`

- [ ] 4.1 `App` 增字段：`occl_check_at: Instant`、`occluders: Vec<[i32;4]>`、`occlusion_enabled: bool`
      （`runtime/mod.rs:214` 附近的构造点同步）
- [ ] 4.2 `runtime/occlusion.rs::update(app: &mut App)`：8 Hz 节流 → 每屏算 bounds
      （由 `mon.blocks` 的 `draw_rect()` + `canvas_origin()`）→ 调 `opaque_rects_above`
      → `rect_fully_covered` → `occlusion_step` → 写回 `Block`。
      **被拖拽的那一块强制 `hit = false`**（design 2.6）
- [ ] 4.3 `runtime/mod.rs` prep 段：在 `collect_monitor_blocks` 循环**之后**、渲染门控**之前**
      调 `occlusion::update(&mut app)`
- [ ] 4.4 读点 1：采集需求扫描的 `if !b.visible { continue; }` 改成 `if !b.visible || b.occluded { continue; }`
- [ ] 4.5 读点 2：`app.rs::collect_monitor_blocks` 里 `DrawBlock` 构造加 `occluded: b.occluded`
      （阶段 2 已加则复核即可）
- [ ] 4.6 读点 3：`mouse_relevant` 的 `.any(|blk| ...)` 谓词加 `!blk.occluded`
- [ ] 4.7 采集段 `just_activated` 为真时 `mon.compositor.invalidate()`（design 2.8，
      **本任务最容易漏、也最容易引入陈旧背景的一处**，注释写清「遮挡恢复不经过 commands，没有兜底」）
- [ ] 4.8 `api/config.rs`：`EngineConfig.occlusion_culling: bool`（Default = `true`）
      + 文档注释说明适用层级（默认 topmost 下收益接近 0，理由见 design 4.4）
- [ ] 4.9 `diag/env.rs`：`EnvOverrides.no_occlusion`（`GLASS_OCCLUSION=0`）
      + 模块文档的例外清单登记一行
- [ ] 4.10 `runtime/mod.rs` 构造期：`let occlusion_enabled = cfg.occlusion_culling && !env.no_occlusion;`
- [ ] 4.11 `api/handle.rs`：`WindowSnapshot.occluded: bool`；`app.rs::publish_snapshot` 的
      `snapshot_key_next` 位掩码从 `visible*2 + interactive` 扩成 `visible*4 + interactive*2 + occluded`
- [ ] 4.12 日志：逐块翻转沿 `debug`、每屏停采/恢复翻转沿 `info`（design 2.10，走级别名，不新增 RAW_TARGETS）
- [ ] 4.13 **验证**：`cargo test --release --workspace` +
      `cargo clippy --workspace --all-targets --release -- -D warnings` +
      `cargo fmt --all -- --check` + `node scripts/check.mjs`
- [ ] 4.14 **debug 构建也要跑一次**（graphics-change-checklist 第 5 问：整数溢出 / Instant 下溢 / NaN）：
      `cargo run -p liquid-verre-playground`（不带 `--release`）

## 阶段 5：取证与文档

> **取证期间不要同时构建**（别的 agent 跑 `cargo build`/`cargo test` 会把机器窗口打成劣化）。
> 先验机器健康：`rel_max < 5 ms`、`acq_max < 25 ms`。

- [ ] 5.1 `python tools/gate_perf.py` 必须 PASS（4 个用例的硬门槛不得回归），记录前后对比表
- [ ] 5.2 BASELINE.md 的 8 个回归脚本全绿：`verify_dirty`（**连跑 3 次以上**）、
      `verify_mouse_isolation`、`verify_validity`、`repro_tremble far`、`repro_tremble many`、
      `verify_adaptive`、`ab_latency`、`ab_memory`
- [ ] 5.3 `tools/verify_occlusion.py`（**主会话负责**，AC1/AC3/AC4/AC5）：
      六个阶段，每阶段「5.3 s 沉降 + 5.3 s 测量」（`[metrics]` 是 5 s 窗口结算，
      必须丢掉跨越状态翻转的那一行）：
      - S0 baseline：不遮挡，`dda_acq > 0`
      - S1 B1 正向：topmost 不透明窗口完整覆盖，`dda_acq == 0`
      - S2 B1 反向（AC5）：topmost `WS_EX_LAYERED` + alpha=128 覆盖，`dda_acq` **不得**下降
      - S3 B1 恢复：移开，`dda_acq > 0`
      - S4 A 正向：无边框铺满屏 + `SetForegroundWindow`，`dda_acq == 0`、`fps` 约 0、
        loop Hz 不超过 ~150（AC3：不退化忙等）
      - S5 A 恢复：关掉全屏窗口，`dda_acq > 0`
      约定：`VERRE_EXE` 可覆盖被测 EXE、显式 `GLASS_DEFAULT_BLOCK=1`、`GLASS_MONITORS=1`、
      `GLASS_ADAPTIVE=0`、`GLASS_METRICS=1`、启动即 `SetCursorPos` 停到左上角并且
      **测量期间绝不移动鼠标**（`mouse_relevant` 只在 `cursor_moved` 时成立）、看门狗超时、
      退出码非 0 表示 FAIL、退出前必须销毁自建窗口并 terminate 子进程
- [ ] 5.4 AC2/AC6 像素取证：`cmp_visual.py` 口径，`GLASS_OCCLUSION=0` vs `=1` 同稳态帧
      （`GLASS_INTERNAL_SHOT_AT_MS=2500`），single 布局四段**全 0**
- [ ] 5.5 `CHANGELOG.md`：Added（`EngineConfig.occlusion_culling` / `WindowSnapshot.occluded` /
      `GLASS_OCCLUSION` / `tools/verify_occlusion.py`）；Changed（`hidden_fs` 停产门、
      `just_activated` 时 invalidate 金字塔账本）；写清「单帧像素逐位不变」与
      「恢复帧多一次整屏 Pass A」
- [ ] 5.6 `README.md`：`GLASS_*` 全表加 `GLASS_OCCLUSION`；「层级」一节说明遮挡剔除的适用范围
- [ ] 5.7 `performance_report.md`：追加本轮（A + B1）的归因与前后数字
- [ ] 5.8 spec 更新（Phase 3.3，**主会话负责**）：
      `backend/directory-structure.md`（新增 `platform/occlusion.rs` / `runtime/occlusion.rs`
      + `unsafe` 分布）、`backend/rendering-pipeline.md`（停产门与 `just_activated` invalidate
      写进逐帧管线契约，确认触及哪几条不变量）、`backend/logging-guidelines.md`（确认没新增 RAW_TARGETS）
- [ ] 5.9 `TODO.md` 末尾的「TODO：遮挡优化」更新为「A + B1 已完成；B2（真停止绘制被遮挡块）
      待评估，前置条件是 `compositor` 的 `prev_rects.len()` 整屏重画判据改成 id 差集」

## 全局验证命令（阶段 5 收尾，必须全绿）

```powershell
cargo test --release --workspace                                  # 基线 154，本任务应 > 154
cargo clippy --workspace --all-targets --release -- -D warnings
cargo fmt --all -- --check
node scripts/check.mjs
python tools/gate_perf.py
```

## 硬约束复核（提交前逐条打勾）

- [ ] `api/` 与 `core/` 的 `unsafe` 数量仍为 **0**
- [ ] 没有新增 `RAW_TARGETS` 前缀
- [ ] 没有改 blur kernel / scissor / 任何幂等外扩常量（`DOWN_GROW` / tent / `BLUR_REACH=96` /
      `HYSTERESIS=192` / `pad = 128 + min(vel*max_lag,128)`）
- [ ] 没有改 cbuffer `c0..c17` 语义、没有新增槽位、`CB_FLOATS` 不变
- [ ] 没有改任何 `LiquidParams::default()` 值
- [ ] 热路径零堆分配（`App.occluders` 跨帧复用；`EnumWindows` 回调里不分配 `String`/`Vec`）
- [ ] `Instant` 相减全部 `checked_sub` / `saturating_duration_since`
- [ ] `GetWindowRgn` 的 HRGN 全部 `DeleteObject`
- [ ] 中文注释无乱码（用本文件顶部的编码配方 + ASCII 自检）
- [ ] **未经用户明确要求不得 git commit / push**

## 风险登记表

| 风险 | 触发条件 | 缓解 | 验收 |
|---|---|---|---|
| 恢复帧露出陈旧背景 | `just_activated` 时没 invalidate（4.7 漏做） | design 2.8 单一来源修复 | AC2 + `verify_validity` 零虚报 |
| 恢复时闪一帧停产前旧画面 | `hidden_fs` 恢复没置 `ready=false` | design 1.3 走既有「采集失效恢复」路径 | AC2 |
| 全透明屏被永久隐藏 | 1.3 忘了 `needs_backdrop` 条件 | design 1.3 明确标注 | transparent-drag 用例仍 PASS |
| 所有屏停产时主循环忙等 | 依赖 `!need_render` 的 8 ms 速率下限 | 不新增代码，但要断言 | AC3（loop Hz 不超过 ~150） |
| GDI 句柄泄漏 | `GetWindowRgn` 忘 `DeleteObject` | 3.5 显式条目 | 长跑 `ab_memory` |
| `EnumWindows` 回调 panic 穿 FFI | 索引越界 / unwrap | 3.6 | debug 构建跑一遍（4.14） |
| 拖拽中被误判遮挡 → 停采 → 卡顿 | 被拖块进入遮挡集合 | design 2.6 强制 `hit=false` | many-drag 阀门不回归 |
| 快照不发布 occluded 翻转 | `snapshot_key` 位掩码没扩 | 4.11 | 上层能读到 `occluded` |
| 中文注释乱码 | 用 PowerShell 管道写文件没设 `PYTHONUTF8` | 本文件顶部的编码配方 | ASCII 自检 OK |
| here-string 被内容提前终止 | 文件内容里出现行首的结束标记 | 顶部「另一个坑」段落 | 写入后做 ASCII 自检 |


---

## 阶段 5 验证状态（2026-10-21，主会话记录）

### 已完成并通过

| 项 | 结果 |
|---|---|
| 阶段 1~4 实现 | 三个 `trellis-implement` 子代理分工完成（core 纯逻辑 / platform Win32 扫描 / runtime+api 接线），写集互不重叠 |
| `trellis-check` 独立验收 | 14 条复核清单逐条通过；**抓到并修掉 4 个真问题**（见下） |
| `cargo test --release --workspace` | **186 passed**（基线 154 → 180 → 186） |
| `cargo test --workspace`（debug，抓溢出/下溢） | pass |
| `cargo clippy --workspace --all-targets --release -- -D warnings` | exit 0，**零 warning**，未加任何 `#[allow]` |
| `cargo fmt --all -- --check` / `node scripts/check.mjs` | exit 0 / 全绿 |
| `api/` + `core/` 的 `unsafe` | 仍为 **0**；`core/` 的 `windows::` 仍为 **0** |
| `RAW_TARGETS` | 仍是 4 个（`boot/verify/metrics/adaptive`），未新增前缀 |
| 中文编码 | 49 个 `.rs` + 9 个文档/脚本全部 UTF-8 无 BOM、`U+FFFD` 计数 0 |
| **AC4 / AC5 / AC7（B1）端到端** | `tools/verify_occlusion.py` 实跑：S0 基线 `dda_acq=53.2`、**S1 不透明全覆盖 `dda_acq=0.0` / `fps=0.0` / `pass_a=0.00` / `loop_hz=120`**、**S2 半透明(layered)覆盖回升到 `dda_acq=53.0`**（AC5 反向对照成立）、S3 回基线 `57.6`；日志翻转沿「桌面采集停产」「遮挡已移开，恢复桌面采集」均命中 |

`trellis-check` 抓到并修掉的 4 个问题：

1. **周期性日志刷屏（真 bug）**：`runtime/occlusion.rs` 的 `log::info!` 写在 `if let Some(b) = bit`
   **外面**，屏下标 ≥32 时 `was` 恒为 `false` ⇒ 每 125 ms 重打一条「桌面采集停产」永不停止，
   而注释声称「静默放弃这一屏的日志」。会把 `[metrics]` 行冲走 ⇒ `perf_probe.py` / `gate_perf.py` 读不到数据、
   阀门瞎掉。已把两条 info 移进状态位分支，并抽出纯函数 `paused_bit(mi)` + 单测。
2. **回调内堆分配（硬约束违背）**：`platform::window::is_own_window` 用 `String::from_utf16_lossy` 比类名，
   而 `EnumWindows` 回调对桌面上**每个**窗口调它一次。已抽出零分配的 `class_name_is_canvas(buf, n)`
   直接比宽字符常量，并把 Win32 返回的长度 clamp 到缓冲区内（切片不越界成为结构性质）+ 单测。
3. **重复判据（R-D2 违背）**：`platform/occlusion.rs` 手写了 `x1 <= x0 || y1 <= y0` 两份。
   裁定**统一**而非豁免 —— 上游给的理由「widen 防溢出」对**纯比较**不成立（比较不会溢出）。
   新增 `core::geometry::rect_is_empty_i64` 作为唯一出处，i32 版委托给它；顺带修正三处把
   「比较可能溢出」写进注释的事实性错误。
4. 「让测试变红」验证：对 4 处判据做定向变异，分别让 11 / 2 / 1 / 4 / 3 条测试变红，
   证明它们不是同义反复；`rect_fully_covered` 的半开区间同时被 8 条手写用例和
   **4000 组随机输入对撞逐像素 oracle** 抓到。

### 2026-10-22 解锁后补跑：全部完成

机器回到交互状态后（前台有窗口、`SetCursorPos`/`GetCursorPos` 正常、DDA 可用）逐项补跑。
完整数字与归因见 `performance_report.md` 的「2026-10-22 第十五轮」。

- [x] **AC1 / AC3 / AC4 / AC5（`tools/verify_occlusion.py`，8/8 PASS）**
      健康窗口实测：S0 基线 `dda_acq=112.1`；**S1 B1 不透明全覆盖 = 0.0 / fps 0.0 / passA 0.00**；
      **S2 半透明(layered)覆盖回升到 113.3**（AC5 反向对照成立）；S3 回基线 115.1；
      **S4 A 全屏（真实形态）= 0.0 / 0.0 / 0.00**；S5 恢复 113.3；
      **S6 A 全屏（layered，与 B1 解耦）= 0.0 / 0.0 / 0.00**；S7 恢复 55.4。
      停产相 `loop Hz` 恒为 119~120 = `IDLE_WAIT_MS`(8 ms) 速率下限 ⇒ **AC3 不退化忙等**。
      S6 是 A 的**独占证明**：layered 窗口已被 S2 证明会被 B1 否决，归零只能来自 A。
- [x] **AC7（`--off`，5/5 PASS）**：同一闪动覆盖窗口下 `GLASS_OCCLUSION=0` 得 `dda_acq=8.1`
      （vs 开开关 0.0）⇒ 开关等价成立；S6/S7 照跑照过 ⇒ **A 无开关**（缺陷修复而非可选优化）。
- [x] **AC2 / AC6 像素取证**（`GLASS_SYNTH_CAP=1` 确定性合成采集源 + 钉死光标 + 稳态帧 4000 ms）：
      `GLASS_OCCLUSION=0` vs `=1` **差异像素 = 0**；A 停产→恢复帧 vs 从未停产的参照帧 **差异像素 = 0**。
- [x] **AC10 BASELINE 回归（最终二进制）**：`verify_dirty` **连跑 3 次全 PASS**、
      `verify_validity` PASS（**金字塔虚报有效 = 0 帧 / 漏画 = 0 次**）、
      `verify_mouse_isolation` PASS、`verify_crossscreen` PASS、
      `repro_tremble far` 连跑 3 次 22/21/22 帧 PASS、`repro_tremble many` 21 帧 PASS、
      `verify_adaptive` PASS（降档 −26.1 MB / 回升 / 显存回收符合预期）、
      `ab_latency` PASS、`ab_memory` PASS（158.6 → 122.3 MB，差 −36.4 MB）。
- [~] **AC9 `gate_perf.py` FAIL，但已证明与本次改动无关**：many-drag 59.2 < 60、lat=2 88.0 < 90。
      三条独立证据：① `GLASS_OCCLUSION=0` 时 many-drag fps **逐位相同（59.2）**，
      而 A 已被 `GLASS_FS_HIDE=0` 关掉；② `just_activated` 的 `invalidate()` 在 gate_perf 里
      **不可能触发**（`drag_path` 振幅让 many/single 两个用例的光标都留在主屏内，跨屏转换不发生）；
      ③ `gfx/` `shaders/` `capture/` **零 diff**。记录的基线本身就是 60.9（贴着门槛），
      脚本自己的 WARN 也写明「受桌面重绘负载 / DWM 背压影响，非渲染成本」。
      **需要在更健康的机器窗口复跑一次**才能拿到 PASS 记录。
- [x] **AC11 / AC12 文档**：`CHANGELOG.md`（`## [0.4.2-unreleased]`）、`README.md`
      （`GLASS_OCCLUSION` / `GLASS_FS_HIDE` 两行 + 层级适用范围 + 管线关键点两行 + 目录树）、
      `TODO.md`（B2 及其前置条件）、`performance_report.md`（第十五轮）、
      `.trellis/spec/backend/{directory-structure,rendering-pipeline,measurement-and-gates}.md`。

### 取证过程中额外发现并修掉的一个问题（不在原计划内）

**闸门不 hermetic**：`fullscreen_app_on` 的形状判据（`WS_POPUP` + 无 caption/thickframe + 铺满屏）
与**无边框最大化的普通应用**（Electron / Tauri：ChatGPT 桌面版、VS Code、app-mode 浏览器）完全重合。
这是既有行为、产品语义也没错，但 A 把命中后果从「画布隐藏」升级成「整屏停产」之后，
`tools/` 的取证脚本会在「前台恰好是这类应用」时拿到 **0 帧连拍并整批假 FAIL**
（实测 `repro_tremble far/many` 0~1 帧、`verify_adaptive` 报「未观察到降档」）。

修法：新增取证后门 **`GLASS_FS_HIDE=0`**（与 `GLASS_ADAPTIVE=0` / `GLASS_OCCLUSION=0` 同构，
只在构造期读一次，`diag/env.rs` 例外清单已登记），给 13 个像素/性能脚本统一注入；
`verify_occlusion.py` **刻意不设**（它就是测 A 的）。另补 `playground/src/panel.rs`
转发 `occluded`（否则 dev 侧无法区分「玻璃不动了」是遮挡剔除、采集失效还是渲染门控）。

⚠ 归因教训（已写进 `measurement-and-gates.md §1.1`）：`repro_tremble` 的帧数在**同一配置内**
就能从 4 帧跳到 17 帧（采集后端处在 DDA `DuplicateOutput 拒绝访问 0x80070005` → 回退 WGC 的
劣化窗口时 `cap` 段单圈阻塞 124 ms）。「帧数少」既可能是代码 bug 也可能是机器窗口，
**必须用「开关关掉 ≈ 改动前行为」做配对 A/B 才能归因**。

⚠ 另一个非复现现象已变成常驻断言：某次 S1 观测到 `prep` 段单圈 **3061 ms**（`loops=2 / win_s=6.12`），
同刺激重跑只有 0.039~0.078 ms。归因未确认（怀疑 USER/GDI 全局锁竞争）。
`verify_occlusion.py` 现在对每个阶段断言 `prep` 单圈中位 ≤ 20 ms，复发即 FAIL。

---

## 追加范围 R-E 的执行记录（2026-10-22）

需求与验收见 `prd.md` 的「R-E 追加范围」与 AC13~AC18；实测数字与真凶定位见
`performance_report.md` 第十五轮「七」。

- [x] R-E1 触发窗口身份进日志：`fullscreen_app_on` → `Option<HWND>`，
      `window_desc(hwnd) -> (class, title, rect)` 只在翻转沿调用（4 Hz 路径零分配）。
      **这一条立刻抓到了真凶**：`class=Progman title="Program Manager" rect=[0,0,5040,2080]`。
- [x] R-E2 cloaked 否决：`display::dwm_cloaked(hwnd) -> Option<bool>` 成为全仓唯一 DWM 调用点，
      `platform/occlusion.rs` 的内联调用删掉改为调它（`unwrap_or(true)` 语义逐字保持）。
      ⚠ 两个调用方对「查询失败」的方向**相反**，所以 helper 返回 `Option` 而不是 `bool`。
- [x] R-E2b（AC18，实测驱动的新增项）：身份排除加 `fg == GetShellWindow()`，
      抽成纯函数 `is_non_candidate_foreground`，`shell` 为 null 时该条自动失效。
- [x] R-E3 `EngineConfig.fullscreen_hide`（默认 `true`）；`fs_hide_enabled = cfg.fullscreen_hide && !env.no_fs_hide`。
- [x] R-E5 纯判据 `is_fullscreen_shape(style, covers_monitor, cloaked)` + 单测。
- [x] 主会话补：`diag/env.rs` 的 `no_fs_hide` 文档同步（原文还写着「产品代码不该关它」，
      现在它对应正式产品开关了）。
- [x] 静态闸门：197 tests、clippy 0 warning（含修掉一个 `clippy::doc_lazy_continuation`：
      文档注释里行首的 `+` 会被 markdown 当列表项，改用顿号）、fmt 0、`check.mjs` 全绿、
      `api/`+`core/` 的 `unsafe` 仍为 0、`RAW_TARGETS` 仍是 4 个。
- [x] AC16 / AC18 / AC7：`verify_occlusion.py` 8/8 PASS、`--off` 5/5 PASS、日志里 Progman 命中 0 次。
- [x] **`repro_tremble far` / `many` / `verify_adaptive`：用最终 R-E 二进制复跑通过**
      （far 18 帧、many 18 帧、adaptive 降档 −26.1 MB / 回升 / 显存回收符合预期）。
      此前那两轮 FAIL（连拍 2~3 帧）确实是 `Progman` 误报 + 机器劣化叠加所致，已排除。
- [x] **`verify_dirty` ×3 / `verify_validity` / `verify_mouse_isolation` / `verify_crossscreen`：
      最终 R-E 二进制复跑全部 PASS**（`verify_dirty` 三次差异像素均为 0，含跨屏两屏）。
      劣化窗口不影响这类「等价关系 / 计数器」判据，只影响帧数与阈值型数字。
- [x] **AC9 `gate_perf`：重启后拿到有效测量**（详见 `performance_report.md` 第十五轮「九」）。
      single-drag **115.2** / p50 8.32、many-drag lat=2 **106.9**、transparent **94.0** / Private 120.8
      ⇒ 三条硬门槛 PASS；`many-drag (lat=1) 59.2 < 60` **FAIL，差 0.8**。
      已证明非本次回归：① `GLASS_OCCLUSION=0` 时同机同会话**逐位相同 59.2**；
      ② `just_activated` 的 `invalidate()` 单独论证为惰性 —— 它**不受**那两个开关控制，
      是 ① 唯一没覆盖到的改动，所以必须单独说：四个场景里 `idle_capturing` 不在测量窗口内翻转，
      只有 warmup 之前那一次边沿；真会触发它的跨屏场景由 `verify_crossscreen` 覆盖并 PASS；
      ③ 基线 60.9 本身只比门槛高 1.5%。
      ⇒ 作为**独立议题**处理（重新标定门槛，或改成相对量），不阻塞本任务收口。
      ⚠ 机器时间线：重启后第 1 轮就是全天最好数据，之后单调恶化（第 3 轮 many 46.7），
      而 `own_us` 始终 32~48 µs ⇒ 是后台任务爬升，不是代码退化。**性能取证要在重启后立刻跑。**

### 收益测量（2026-10-22，AC1~AC5 的量化补强）

- 玻璃被完整遮住时总开销 **11.36% -> 0.95% CPU 单核（-92%，约 12 倍）**。
- B1 单独（同场景开关对照）**1.47% -> 0.95%（-35%）**，dda 10.8/s -> 0、passA 1.19 -> 0 Mpx，两轮重复同向。
- ⚠ 反直觉发现并修正了表述：**停采不是无条件更省** —— 桌面静止时停采会让主循环落到 8 ms
  睡眠下限（125 次/秒唤醒），比继续「阻塞在内核等帧」略贵（1.17% vs 0.68%）。
  正确表述：A/B1 省的是「桌面在变而玻璃看不见」那部分工作。详见 performance_report 第十五轮「十」。
- ⚠ 第一版测量脚本因用 	ime.sleep 未驱动覆盖窗口闪动，测的是「无活可省」的场景，
  结论符号相反 —— 已作废并重测。教训写入报告「测量方法教训」段。

---

## 追加范围 R-F：性能阀门改成两档判定（2026-10-22，用户指定"改成相对量"）

- [x] `gate_perf.py`：相对 A/B（`VERRE_OLD_EXE`）为硬判据 + 绝对门槛回退；`Private` 两档都硬判。
- [x] `spread_pct` 噪声可辨识度：散布 >= 劣化 x 0.8 ⇒ 判「结果不可信」并 FAIL（不宣布回归、不放行）。
- [x] `REPS` 默认 3（对齐 spec 的 >=3 重复口径；2 轮实测被 17% 噪声骗过）。
- [x] 新增 `tools/test_gate_logic.py`：**19 条判定路径**全 PASS（纯逻辑、秒级、不启动 EXE）。
- [x] 参考二进制：0.4.1 干净构建已存 `D:\github-project\gate-ref\liquid-verre-playground-0.4.1.exe`，
      worktree 已清理；建法写进 `measurement-and-gates §4.4`。
- [x] `CHANGELOG.md` / `performance_report.md`「十一」/ `measurement-and-gates §4` 已同步。
- [x] 静态闸门：`node scripts/check.mjs` 全绿。
- [ ] **绝对门槛重新标定**：故意**没做** —— 相对档已接管，就地放宽绝对值等于削弱闸门。
      等相对档跑出几份稳定记录后再按新机器实测重标绝对表。
- [ ] **`gate_perf` 需要一个健康窗口才能出结论**：今天最后一次尝试臂内散布 22~130%，
      已由新逻辑正确判为「不可信」。**重启后第一件事就跑它**（命令见 `RUNBOOK.md`）。
