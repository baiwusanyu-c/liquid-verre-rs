# 遮挡优化 A + B1：隐藏屏停产 + 被遮挡块不进触发源

## Goal

把「玻璃被别的窗口完全遮住」这件事变成**可测量的性能收益**，同时不改变任何一帧的绘制内容。

拆成两项（用户 2026-10-21 明确选择 A + B1，B2 暂不做）：

- **A｜全屏遮挡整屏 → 整屏停产**
  `MonitorRt::hidden_fs` 目前只是「可见性位」：画布 `SW_HIDE` 了，但采集、Pass A、Pass B、
  Present、Commit 全部照跑。无边框窗口全屏（Win10/11 上绝大多数游戏与视频播放器的形态，
  也正是 `platform::display::fullscreen_app_on` 判的那一类）时 DDA 一切正常，于是我们
  满频跟着一块**看不见的画布**渲染。A 把 `hidden_fs` 升级为「停产门」。

- **B1｜非全屏窗口完全遮挡单块 → 该块不再驱动采集与重画**
  新增逐块遮挡检测（z 序在我们之上、几何上完整覆盖该块**绘制矩形**的不透明窗口）。
  被遮挡的块：① 不再计入「本屏需要桌面采集」；② 不再计入 `mouse_relevant`。
  **不改绘制集合**（`mon.blocks` / `cur_rects` 原样），因此单帧像素输出逐位不变。

## Background（为什么这两项值钱，用项目自己的实测数字）

`performance_report.md` 2026-10-21 健康窗口阀门：

| 用例 | fps | p50 | CPU% | DDA 次/秒 | own µs（A+B+Commit） |
|---|---|---|---|---|---|
| single-drag | 110.2 | 8.34 ms | 11.87 | 76 | 28 |
| many-drag | 60.9 | 16.66 ms | 16.25 | 107 | 76 |
| transparent-drag | 93.8 | 8.34 ms | 9.69 | **0** | 20 |

两条归因决定了收益落在哪：

1. **CPU 大头在采集侧**：`own` 只有 20~76 µs/帧（8.33 ms 预算的 ~1/100）；
   transparent-drag 把采集整条停掉（DDA 0 次/秒）就把 CPU 从 16% 量级拉到 9.69%。
   → A 与 B1 的主要收益都是「让采集停下来」，量级对齐 transparent-drag。
2. **GPU 大头在 Pass A + Pass B 合起来**：baseline 77 fps、skip-PassA→110、skip-PassB→114。
   → A 让整屏 GPU 帧时间归零；B1 不改绘制集合，**不宣称** GPU 收益（那是 B2 的范围）。

## Requirements

### R-A 全屏遮挡停产

- **R-A1** `hidden_fs == true` 的屏：跳过采集（不调 `try_capture`）、跳过渲染
  （不调 `render_monitor`）。该屏 CPU 采集开销与 GPU 帧时间都归零。
- **R-A2** 停产期间主循环**不得**退化成忙等：现有的 `!need_render` 分支已有
  `IDLE_WAIT_MS`（8 ms）速率下限，必须继续生效（前科：DDA ACCESS_LOST 时 96 kHz / 83% CPU）。
- **R-A3** 恢复（`hidden_fs` true→false）时**不许闪旧背景、不许呈现陈旧帧**：
  沿用「采集失效恢复」那条已验证路径 —— `compositor.invalidate()` + `ready = false`
  （仅当 `needs_backdrop`），画布保持隐藏，直到拿到**新的**采集帧才 `ready = true` + 显示。
- **R-A4** 拖拽期间的现有保护不变：`fullscreen_app_on` 的判定已带 `!dragging`，
  拖拽中不得隐藏/停产（隐藏会释放 `SetCapture` 打断拖拽状态机）。
- **R-A5** 真独占全屏（DDA `ACCESS_LOST` → `hidden_capture` + `ready=false`）那条路径
  行为不变，不得重复隐藏/重复日志。

### R-B1 逐块遮挡检测

- **R-B1-1 判据（保守优先）**：块被判定为「遮挡」当且仅当它的**绘制矩形**
  （几何矩形 ⊕ 阴影外扩，即 `DrawBlock::draw_rect()` 换算到虚拟屏绝对坐标）
  被「z 序在我们之上、且被认定为不透明」的窗口矩形的**并集完整覆盖**。
  理由：Pass B 是 per-block viewport + scissor，绘制矩形是该块可见像素的**硬边界**；
  完整覆盖 ⇒ 该块贡献 0 个可见像素 ⇒ 它的一切触发源都可证明是浪费。
- **R-B1-2 遮挡者资格**：`EnumWindows` 自顶向下枚举，遇到**本屏自己的画布**停止；
  跳过自家任意画布（`platform::window::is_own_window`）；跳过不可见 / 最小化 /
  cloaked（`DWMWA_CLOAKED`）/ `WS_EX_LAYERED` / `WS_EX_TRANSPARENT` /
  带窗口区域（`GetWindowRgn != ERROR`）/ 零面积的窗口。**任何不确定一律判「不遮挡」**。
- **R-B1-3 成本**：检测**不得**进每帧热路径。8 Hz 轮询（125 ms），复用缓冲区零堆分配；
  昂贵的 `DwmGetWindowAttribute`（跨进程到 DWM）只对「便宜的矩形相交测试已通过」的窗口调用。
- **R-B1-4 迟滞**：置位需连续 2 次判定命中（`OCCLUSION_CONFIRM_POLLS = 2`，即 ≤250 ms）；
  **解除只需 1 次**（立即恢复）。错判方向必须是「多渲染」而不是「少渲染」。
- **R-B1-5 消费点只有两处**：
  ① 主循环顶部的采集需求扫描（`active` / `wants_bd`）跳过 `occluded` 块 —— 于是
  「本屏所有液态块都被遮挡」会走现成的 `needs_backdrop_now == false` → `idle_capturing`
  停采路径（与全透明屏同一条，需求 F3.1 的既有形态）；
  ② `mouse_relevant` 的 `.any(...)` 谓词加 `!blk.occluded` —— 于是光标扫过一块
  看不见的玻璃不会把 `capture_wait_ms` 打到 0（不再让采集从「驱动里阻塞」退化成满频轮询）。
- **R-B1-6 不改的东西**：`mon.blocks` / `mon.order` / `cur_rects` / `prev_rects` /
  compositor 的脏区判据 / `set_click_through` / hover 与命中测试 / cbuffer `c0..c18`。
  **单帧绘制内容逐位不变**。
- **R-B1-7 开关**：`EngineConfig.occlusion_culling: bool`（默认 `true`）+
  `GLASS_OCCLUSION=0` 关闭（取证 A/B 用）。关掉时行为与今天**逐位一致**。

### R-C 可观测性与 API

- **R-C1** `WindowSnapshot` 增加 `occluded: bool`（上层能解释「我的玻璃为什么不动了」）。
  值类型不加 `#[non_exhaustive]`（quality-guidelines 硬规矩）。
- **R-C2** 日志走级别名（`log::info!` / `log::debug!`），**不得**新增 `RAW_TARGETS` 前缀
  （前缀即接口，改前缀 = 打断质量阀门）。逐块翻转沿走 `debug`；
  「本屏因全部块被遮挡而停采」的翻转沿走 `info`（稀有且有意义）。
- **R-C3** 热路径禁止格式化日志；翻转沿触发，不得每帧打印。

### R-D 分层与工程规矩

- **R-D1** 新增 Win32 枚举代码放 `platform/occlusion.rs`（`unsafe` 只允许集中在
  `platform/` `gfx/` `capture/` + `runtime/` 的既有平台点位）。
- **R-D2** 纯几何（矩形并集覆盖判定、迟滞状态机）放 `core/`（零 `windows::`、零 `unsafe`、可单测）。
  矩形运算单一来源 = `core/geometry.rs`，禁止在 `platform/` 或 `runtime/` 再手写一份。
- **R-D3** `api/` 与 `core/` 必须保持 0 个 `unsafe`。
- **R-D4** `GetWindowRgn` 返回的 `HRGN` 必须 `DeleteObject`（GDI 句柄泄漏是静默的）。
- **R-D5** `Instant` 相减一律 `checked_sub` / `saturating_duration_since`。

### R-E 追加范围（2026-10-22）：`fullscreen_app_on` 的误触发

阶段 5 取证时发现：`platform::display::fullscreen_app_on` 的形状判据（**前台**窗口 =
`WS_POPUP` 且无 caption/thickframe 且铺满整块屏）**不看 cloaked、不看 z 序、也不看那到底是不是
游戏**。实测确实误触发过（脚本没有创建任何窗口、只移动光标并点击，引擎两次打出
「检测到独占全屏应用，暂时隐藏玻璃」，导致 `repro_tremble` 只拿到 0~1 帧连拍）。
当时**没有记录是哪个窗口**触发的 —— 因为那条日志根本不打窗口身份。

这是**既有行为**，不是 A/B1 引入的；但 A 把命中后果从「画布隐藏」升级成「整屏停产」，
放大了它的杀伤力。本任务已经用取证后门 `GLASS_FS_HIDE=0` 让闸门恢复 hermetic，
但那只是绕过，不是修。追加三项**零风险**修复：

- **R-E1 触发窗口身份进日志**：命中时把该窗口的 `class` / `title` / `rect` 打进那条 `info`。
  只能在**翻转沿**构造字符串（不在 4 Hz 轮询里），因此 `fullscreen_app_on` 改为返回
  `Option<HWND>`（命中的那个前台窗口），另加 `window_desc(hwnd)` 供翻转沿调用 ——
  这样记录的是**真正触发判定的那个窗口**，而不是「打日志时恰好在前台的那个」。
- **R-E2 cloaked 否决**：cloaked 的窗口一个像素都不画（UWP 挂起 / 被 DWM 隐藏），
  不可能是「独占全屏应用」。⚠ **保守方向与 B1 相反**：`DwmGetWindowAttribute` 查询失败时
  一律当**未 cloaked**，即维持既有判定 —— 隐藏玻璃才是破坏性动作，
  只有在**确知** cloaked 时才否决，这样对真游戏零影响。
- **R-E3 `EngineConfig.fullscreen_hide: bool`（默认 `true`）**：把 `GLASS_FS_HIDE` 从
  「取证后门」升级成上层应用可用的正式开关，与 `adaptive_quality` / `occlusion_culling` 同构。
  生效判据 `cfg.fullscreen_hide && !env.no_fs_hide`，构造期算一次。
- **R-E4 单一来源**：DWM cloaked 查询抽成一个共享 helper，`platform/occlusion.rs` 与
  `platform/display.rs` 都用它。**返回 `Option<bool>` 而不是 `bool`**，因为两个调用方对
  「查询失败」的处置方向相反（遮挡扫描：失败 ⇒ 当 cloaked ⇒ 不算遮挡者；
  全屏检测：失败 ⇒ 当未 cloaked ⇒ 维持既有行为）。
- **R-E5 纯判据可单测**：把「形状 + cloaked ⇒ 是否算独占全屏」抽成纯函数
  （入参是已查得的 `style` / `covers_monitor` / `cloaked: Option<bool>`），
  真实 HWND 无法在 CI 里构造，判据本身必须能单测。

### R-E 的验收标准

- [ ] **AC13（可诊断）** 命中时日志含触发窗口的 `class` / `title` / `rect`；
      字符串只在翻转沿构造，4 Hz 轮询路径**零分配**（`fullscreen_app_on` 返回 `Option<HWND>`）。
- [ ] **AC14（cloaked 否决 + 保守方向）** 纯判据单测覆盖：
      `WS_POPUP` + 无 caption/thickframe + 铺满屏 + `cloaked=Some(false)` ⇒ true；
      同样条件 + `cloaked=Some(true)` ⇒ **false**；
      同样条件 + `cloaked=None`（查询失败）⇒ **true**（维持既有行为，不误伤真游戏）；
      有 caption / 非 popup / 未铺满屏 ⇒ false。
- [ ] **AC15（开关）** `EngineConfig.fullscreen_hide = false` 时整块 4 Hz 检测被跳过
      （`hidden_fs` 永不置位），且默认值为 `true`；`GLASS_FS_HIDE=0` 仍然有效（两者取与）。
- [ ] **AC16（不回归 R-16）** `tools/verify_occlusion.py` 的 S4/S6（真·无边框全屏窗口，
      未 cloaked）仍然 PASS —— 即修复不能把 R-16 的正当场景一起废掉。
- [ ] **AC18（实测误报源：桌面本身）** 前台是 `Progman`（"Program Manager"，即 Windows 桌面）时
      **不得**判为独占全屏。这是 R-E1 的身份日志当场抓到的：`Progman` 的样式是 `WS_POPUP`
      且无 caption/thickframe、rect 铺满整个虚拟屏（实测 `[0, 0, 5040, 2080]`），
      **三条形状判据全中**；而「没有别的窗口持有焦点」时它就是前台窗口 ——
      用户点一下壁纸或全部最小化，玻璃就消失并整屏停产。cloaked 否决挡不住它（它没被 cloak），
      必须在**身份**这一层排除（`GetShellWindow()`）。排除规则要抽成纯函数以便单测，
      且 `GetShellWindow()` 失败时必须退化成修复前行为，**不许**把「所有窗口」都当成桌面。
- [ ] **AC17** 静态闸门全绿（test / clippy `-D warnings` / fmt / `node scripts/check.mjs`），
      `api/` + `core/` 的 `unsafe` 仍为 0，`RAW_TARGETS` 仍是 4 个。

### 明确不做（本轮）

- **不用「桌面更新速率」区分游戏与静态应用**：需要调阈值，且暂停/菜单画面的真游戏
  重绘率也很低 ⇒ 会漏检，属于要单独取证的行为变更。
- **不用「该窗口必须在画布之上」当判据**：真游戏在无边框全屏时同样位于我们 TOPMOST 画布之下，
  加了这条等于把 R-16 整个废掉（这个方向我先前提过，是错的，已撤回）。
- **不给 `EngineEvent::ExclusiveFullscreen` 加字段**：那是 breaking change（变体字段变更
  会让上层的 `match` 编译失败），身份走日志即可。

## Non-Goals（本任务明确不做）

- **B2**：真正停止绘制被遮挡的块（改 `cur_rects` / 脏区集合）。理由：
  `gfx/compositor.rs` 的 `prev_rects.len() != cur_rects.len()` → **整屏 full redraw**，
  剔除状态每次翻转都要付一次整屏 Pass A + Pass B；且消失块的旧像素不会被擦（残影）。
  收益只在「遮挡是常态」（`AlwaysOnBottom`）时才盖得过这些代价。留待独立任务。
- `SetWinEventHook` 事件驱动即时恢复。当前用 8 Hz 轮询，恢复延迟 ≤125 ms，
  代价是「遮挡移开后最多 125 ms 内玻璃里是旧桌面」。若将来要亚帧恢复再加钩子。
- 默认层级（`AlwaysOnTop`）下不引入任何新行为差异：扫描会在自家画布处立即停止，
  遮挡者集合几乎恒为空（除非别的 topmost 窗口盖住我们，例如任务栏完全盖住某块）。

## Acceptance Criteria

- [ ] **AC1（A 停产生效）** 无边框铺满屏的前台窗口存在时，`[metrics]` 的
      DDA 次/秒 = **0**、own µs ≈ 0、fps ≈ 0；该窗口关闭后 ≤1 s 内恢复
      （DDA > 0，玻璃像素正确）。
- [ ] **AC2（A 不闪旧帧）** 恢复瞬间不得呈现停产前的陈旧桌面：
      取证脚本对比「恢复后稳态帧」与「从未停产的稳态帧」，`cmp_visual.py` 口径
      single 布局四段（deep/interior/ring/outside）**全 0**。
- [ ] **AC3（A 不忙等）** 全屏遮挡 + 无任何触发源时，主循环频率不超过
      `1000 / IDLE_WAIT_MS` 量级（CPU 单核占用个位数），复用现有速率下限单测口径。
- [ ] **AC4（B1 停采生效）** 单块被不透明窗口完整覆盖后，`[metrics]` DDA 次/秒降到 **0**；
      覆盖窗口移开后 ≤1 s 恢复。
- [ ] **AC5（B1 反向：不许错剔）** 用**半透明**（`WS_EX_LAYERED` + alpha<255）窗口
      完整覆盖同一块玻璃时，DDA 次/秒**不得**下降（该窗口必须被判为「不遮挡」）。
- [ ] **AC6（B1 不改像素）** `GLASS_OCCLUSION=0` 与 `=1` 在**同一稳态帧**下
      `cmp_visual.py` single 四段全 0（绘制集合未变）。
- [ ] **AC7（关开关等价）** `occlusion_culling = false`（或 `GLASS_OCCLUSION=0`）时，
      B1 的两个消费点都短路，行为与改动前逐位一致。
- [ ] **AC8（单测）** 新增纯函数单测全绿：矩形并集完整覆盖判定（含 L 形/拼接/部分覆盖/
      零面积/相邻边接触）、遮挡者资格判定的纯逻辑部分、迟滞状态机（置位需 2 次、解除 1 次）。
- [ ] **AC9（既有闸门全绿）** `cargo test --release --workspace`、
      `cargo clippy --workspace --all-targets --release -- -D warnings`、
      `cargo fmt --all -- --check`、`node scripts/check.mjs` 全过；
      `python tools/gate_perf.py` PASS（不得让任一硬门槛回归）。
- [ ] **AC10（BASELINE 回归）** 8 个回归脚本全绿，其中 `verify_dirty` **连跑 ≥3 次**
      （间歇残差只在多次运行里露头）、`verify_validity` 不得报「金字塔虚报有效」或「漏更新」。
- [ ] **AC11（文档）** `CHANGELOG.md` 记 Added/Changed；`README.md` 的 `GLASS_*` 全表加
      `GLASS_OCCLUSION`、层级一节说明遮挡剔除的适用范围；`.trellis/spec/` 相应更新。
- [ ] **AC12（TODO 清理）** 根 `TODO.md` 末尾的「TODO：遮挡优化」按最终结论更新
      （A + B1 已做，B2 记为待评估项及其前置条件）。

## Constraints

- 不改 blur kernel / scissor / 任何幂等外扩常量（`DOWN_GROW=[14,12,8,0]`、tent `32/24/16/8`、
  `BLUR_REACH=96`、`HYSTERESIS=192`、`pad = 128 + min(vel*max_lag,128)`）。
- 不改 cbuffer `c0..c17` 语义（冻结），不新增 cbuffer 槽位。
- 不改任何默认参数值（`LiquidParams::default()` 就是视觉基线）。
- 热路径零堆分配（复用 `App` 里的缓冲区）。
- 未经用户明确要求不得 `git commit` / `push`。
- 性能取证期间**不得**同时构建（别的 agent 跑 `cargo build`/`cargo test` 会把窗口打成劣化）。
