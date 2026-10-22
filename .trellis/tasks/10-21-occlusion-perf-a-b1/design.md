# 技术设计：A 隐藏屏停产 + B1 被遮挡块不进触发源

> 所有 `src/...` 路径相对 `crates/liquid-verre/`。

## 0. 现状事实（改之前必须认账的四条）

| # | 事实 | 位置 |
|---|---|---|
| F1 | `fullscreen_app_on()` 4 Hz 轮询 → `mon.hidden_fs` → `apply_visibility()` → `SW_HIDE`，并发 `EngineEvent::ExclusiveFullscreen` | `platform/display.rs:116`、`runtime/mod.rs:717-735` |
| F2 | **采集段不读 `hidden_fs`**：本屏有块 → `has_blocks=true` → 每圈 `try_capture`；拖拽时 `capture_wait_ms=0` | `runtime/mod.rs:340-400` |
| F3 | **渲染段唯一的跳过条件**是 `!ready && needs_backdrop`；`hidden_fs` 的屏 `ready` 仍是 `true` → Pass A + Pass B + Present + Commit 照跑在看不见的画布上 | `runtime/mod.rs:820` |
| F4 | 已存在一条**成熟的停采路径**：`needs_backdrop_now == false` → `idle_capturing = true; continue`；恢复那一帧 `just_activated = has_blocks && idle_capturing` → `wait = 32` 阻塞取新帧（全透明屏 / 空屏走的就是它，transparent-drag 的 DDA 0 次/秒由此而来） | `runtime/mod.rs:370-400` |

设计原则：**A 与 B1 都复用 F4 这条已验证路径**，不新造停采机制。差别只在「谁把
`needs_backdrop_now` / 采集循环短路掉」，以及「恢复时账本怎么保证诚实」。

---

## 1. A：`hidden_fs` 从可见性位升级为停产门

### 1.1 采集段（`runtime/mod.rs` 每屏循环，紧接 `copied_px = 0` 之后）

```rust
mon.new_backdrop = false;
mon.backdrop_dirty = None;
mon.copied_px = 0;
// 停产门 A：画布被独占/无边框全屏应用压住 → 整屏不采集。
// 必须置 idle_capturing：恢复那一帧才会走 just_activated → wait=32 阻塞取新桌面。
if mon.hidden_fs {
    mon.idle_capturing = true;
    continue;
}
```

- `continue` 在 `has_blocks` 计算之前，因此 `active` / `wants_bd` 对本屏的结论被完全忽略，
  不会把别的屏的 `overflow` 语义搞乱（`overflow` 是「块数超过 16 屏上限」的兜底，保持不变）。
- **R-A2 忙等**：停产屏不产生 `new_backdrop` → 若所有屏都停产，`need_render == false` →
  走 `!need_render` 分支的 `IDLE_WAIT_MS`（8 ms）速率下限睡眠。现有机制已覆盖，**不需要新代码**，
  但必须写取证断言（AC3）。

### 1.2 渲染段（`runtime/mod.rs:820`）

```rust
if app.monitors[mi].hidden_fs {
    continue;
}
if !app.monitors[mi].ready && app.monitors[mi].needs_backdrop {
    continue;
}
```

顺序：`hidden_fs` 判定放在 `ready` 判定**之前**，语义是「这块屏这一帧根本不参与」。
`render_monitor` 不被调用 → `presented` 不置位 → `quality.tick(own_us, false)`，
自适应控制器看到的是一个「零负载」窗口，不会误降档。

### 1.3 恢复边沿（`runtime/mod.rs:717-735` 的 `if hit != mon.hidden_fs`）

```rust
if hit != mon.hidden_fs {
    mon.hidden_fs = hit;
    if !hit {
        // 停产期间桌面变了而我们没采集：金字塔与 present 账本都不可信。
        mon.compositor.invalidate();
        // 需要桌面背景的屏：走「采集失效恢复」同一条路径 —— 保持隐藏，
        // 直到拿到**新的**采集帧才 ready=true + 显示（R-A3：零陈旧帧）。
        if mon.needs_backdrop {
            mon.ready = false;
        }
    }
    window::apply_visibility(mon);
    ...
}
```

为什么这样是对的：

- `invalidate()` = `pyramid_valid=None` + `background_valid=false` + `prev_rects.clear()` +
  `has_presented=false` → 下一帧 `dirty = full`，整屏 Pass A + 全量重画。这正是
  「采集源重建后」已有的同一处理（`runtime/mod.rs:452-458`），语义单一来源。
- `ready = false` 后 `apply_visibility()` 会 `SW_HIDE`（它的判据含 `mon.ready`），
  画布继续隐藏；下一次采集到 `Ok(Some(...))` 时命中既有的
  `if !mon.ready && mon.compositor.capture_ready() { mon.ready = true; hidden_capture = false; apply_visibility(mon); }`
  → 自动显示。**零新状态机**。
- `if mon.needs_backdrop` 这个条件不可省：全透明屏没有采集可等，把 `ready` 打成 false
  会让它**永远不回来**（透明模式不采样桌面，本来就没有陈旧问题）。

代价（诚实记录）：玻璃在全屏应用退出后**晚约一帧 + 一次 32 ms 阻塞采集**才出现。
换来的是「绝不呈现停产前的陈旧桌面」。这是有意的取舍（R-A3 优先于 40 ms 延迟）。

### 1.4 不动的东西

- `hidden_empty`：已经零成本（采集被 `needs_backdrop_now=false` 短路、渲染被
  `cur_rects.is_empty() && prev_rects.is_empty()` 短路）。
- `hidden_capture`：`mon.capture = None` → 采集段自然落到 `retry_at` 退避分支；
  `ready=false` → 渲染段已跳过。
- `update_region` / `set_click_through`：拖拽时非拖拽屏已有 `region_idle` 跳过；
  隐藏窗口上这两个调用的成本可忽略，不在本任务扩大改动面。

---

## 2. B1：逐块遮挡检测

### 2.1 数据流（单一来源 = `Block::occluded`）

```
8 Hz: runtime::occlusion::update(app)          ← 唯一写点
        └─ 每屏: 由 mon.blocks(DrawBlock) 求绝对绘制矩形 + 外接盒
        └─ platform::occlusion::opaque_rects_above(mon.hwnd, bounds, &mut app.occluders)
        └─ core::geometry::rect_fully_covered(draw_rect_abs, &app.occluders)
        └─ core::block::occlusion_step(streak, hit) → (occluded, streak)
        └─ 写回 manager 的 Block.occluded / Block.occl_streak

读点 1（每圈，采集段顶部）: active/wants_bd 扫描跳过 b.occluded
读点 2（每圈，collect_monitor_blocks）: DrawBlock.occluded ← b.occluded
读点 3（每圈，mouse_relevant）: .any(|blk| !blk.occluded && 命中影响盒)
```

时序：写点在 prep 段（`collect_monitor_blocks` 之后、渲染门控之前），因此读点 2/3 拿到的是
**上一轮**扫描结果，滞后 ≤1 圈（≤8 ms）+ 轮询间隔。对 8 Hz 决策无意义，接受。
好处：`Block.occluded` 是唯一真值，不存在两处状态漂移（spec 的「单一来源」硬规矩）。

### 2.2 `core/geometry.rs`：矩形并集完整覆盖（纯函数，零 Win32，可单测）

```rust
/// `target` 是否被 `covers` 的并集**完整**覆盖（半开区间 [x0,y0,x1,y1]）。
#[must_use]
pub fn rect_fully_covered(target: [i32; 4], covers: &[[i32; 4]]) -> bool
```

- 分割法：把 target 切成「尚未被覆盖」的矩形列表，逐个 cover 做差集
  （一个矩形减一个矩形最多产生 4 个子矩形）。列表空 ⇒ 完整覆盖。
- 空 `covers` → `false`；**零面积 target**（`x1<=x0 || y1<=y0`）→ `true`（写进文档注释）。
- 相邻边接触（`cover.x1 == target.x1`）必须算覆盖：半开区间语义，不许差 1 px。
- 只在 8 Hz 轮询里调用，**不在每帧热路径**，允许一次 `Vec` 分配（文档注释写明理由）。
- 复用现有矩形工具风格；**不要**在别处再写一份矩形减法。

### 2.3 `core/block.rs`：迟滞状态机（纯函数，可单测）

```rust
/// 遮挡确认所需的连续命中次数（8 Hz 轮询 ⇒ ≤250 ms）。
pub(crate) const OCCLUSION_CONFIRM_POLLS: i8 = 2;

/// 返回 (新的 occluded, 新的 streak)。
/// 置位需连续 OCCLUSION_CONFIRM_POLLS 次命中；**解除只需 1 次未命中**（立即恢复）。
#[must_use]
pub(crate) fn occlusion_step(streak: i8, hit: bool) -> (bool, i8)
```

`Block` 增加两个字段：`pub occluded: bool`、`pub occl_streak: i8`（都是 `Copy` 标量，
不破坏「热路径零堆分配」）。`Block::new` 初始化为 `false / 0`。streak 到阈值即饱和，
**不许溢出 i8**（debug 构建会 panic）。

### 2.4 `core/draw.rs`：`DrawBlock` 增加 `pub occluded: bool`

投影时从 `Block::occluded` 拷贝。`mouse_relevant` 读这一份。
必须同步 `core/draw.rs` 的单测构造函数 `mk()` 与 `runtime/app.rs` 里 `DrawBlock` 的字面量，
否则编译不过。

### 2.5 `platform/occlusion.rs`：Win32 扫描（新增，`unsafe` 集中在此）

```rust
pub(crate) fn is_opaque_occluder(exstyle: u32, visible: bool, iconic: bool,
                                 has_rgn: bool, cloaked: bool) -> bool
pub(crate) fn opaque_rects_above(stop_at: HWND, bounds: [i32; 4], out: &mut Vec<[i32; 4]>) -> usize
```

`EnumWindows` 回调里的判定顺序（**便宜的在前，贵的在后**，这是 R-B1-3 的成本控制核心）：

1. `hwnd == stop_at` → **返回 FALSE 终止枚举**（再往下的窗口盖不住这块屏的画布）。
2. `window::is_own_window(hwnd)` → 跳过但**继续**枚举（自家另一块屏的画布可能排在本屏画布之上；
   它们几何上不重叠，但语义上必须跳过）。
3. `GetWindowRect` 失败 / 零面积 / 与 `bounds` 不相交 → 跳过。
4. `IsWindowVisible` / `IsIconic` / `GWL_EXSTYLE` / `GetWindowRgn` → 收集布尔量，
   统一交给 `is_opaque_occluder` 判定（判据集中一处，可单测）。
5. ⚠ `GetWindowRgn(hwnd) != ERROR` 时**必须 `DeleteObject`** 返回的 HRGN
   （R-D4：静默 GDI 句柄泄漏，8 Hz 下几分钟就耗尽）。
6. **只有前面全部通过的候选**才调 `DwmGetWindowAttribute(DWMWA_CLOAKED)`
   （唯一跨进程调用，放最后）；调用失败按 `cloaked = true` 处理（保守）。
7. 通过全部关卡 → `out.push([left, top, right, bottom])`。

⚠ **`EnumWindows` 回调里 panic 穿越 FFI 边界是 UB**：回调必须写成不会 panic 的形式
（无索引越界、无 `unwrap` / `expect`、无整数溢出），或用 `catch_unwind` + `AssertUnwindSafe` 兜住。
本项目锁 `windows = "0.58"`；常量未导出时本地 `const` 定义并注明（前科：`MONITORINFOF_PRIMARY`）。
回调里**不得**分配 `String` / `Vec`（`out` 由调用方传入复用）。本模块**不打日志**
（8 Hz 轮询，日志由 `runtime::occlusion` 的翻转沿负责）。

`bounds` 的计算：本屏所有 `mon.blocks` 的 `draw_rect()` 并集，平移到绝对坐标
（`+ mon.canvas_origin()`）。若本屏无块 → 直接返回，不调 `EnumWindows`。

### 2.6 `runtime/occlusion.rs`：编排（新增，薄薄一层）

```rust
/// 8 Hz 轮询：更新每块的 occluded 状态。
pub(crate) fn update(app: &mut App)
```

- 频率由 `App.occl_check_at: Instant` 控制（与 `fs_check_at` 同构，独立常量
  `OCCLUSION_POLL: Duration = 125 ms`）。
- **被拖拽的那一块强制 `hit = false`**：拖拽中的块必然在鼠标下可见，若被判遮挡会停采 → 拖拽卡顿，
  这是最难查的间歇问题，直接在源头排除。
- `occlusion_enabled == false` 时函数体第一行 `return`（R-B1-7 / AC7）。
- 缓冲区 `App.occluders: Vec<[i32; 4]>` 跨帧复用，每屏 `clear()`。

### 2.7 消费点改动（`runtime/mod.rs`）

**读点 1｜采集需求扫描**（顶部 `for b in app.manager.blocks()`）：

```rust
if !b.visible || b.occluded {   // ← 新增 b.occluded
    continue;
}
```

后果链：某屏所有液态块都被遮挡 → `wants_bd[mi] = false` → `needs_backdrop_now = false`
→ **F4 那条既有路径**：`idle_capturing = true; continue` → 采集整条停摆（DDA 0 次/秒）。
恢复时 `just_activated = true` → `wait = 32` 阻塞取新桌面。

**读点 3｜`mouse_relevant`**：

```rust
if cursor_moved
    && mon.blocks.iter().any(|blk| {
        !blk.occluded && blk.x < x1 && blk.x + blk.width > x0 && ...
    })
```

这是 B1 里**单位收益最高**的一处：`mouse_relevant` 不仅触发整帧重画，还把
`capture_wait_ms` 打到 **0** → 采集从「在驱动里阻塞」退化成满频轮询 → CPU 直接上去
（11.87% / 16.25% 那一档的成因之一）。

### 2.8 恢复时的账本诚实性（B1 唯一的新增正确性风险）

F4 路径的现有注释说「金字塔账本由 commands 层的 invalidate 兜底」——
那是因为**透明↔液态切换必然经过 `Command`**。遮挡恢复**不经过 commands**，没有兜底：
停产期间桌面变了而我们没采集，`pyramid_valid` 仍声称某区域有效 ⇒ 恢复后可能露出陈旧背景
（这正是 `verify_validity` 抓的那类「虚报有效」前科）。

修法（单一来源，不新开状态位）：在采集段 `just_activated` 为真的那一帧加一次 invalidate。

```rust
let just_activated = has_blocks && mon.idle_capturing;
mon.idle_capturing = !has_blocks;
if just_activated {
    // 停产期间桌面可能已变，而金字塔/present 账本仍声称有效 ⇒ 一律作废，
    // 让恢复的第一帧走整屏 Pass A + 全量重画。与「采集源重建」同一处理。
    mon.compositor.invalidate();
}
```

影响面（必须显式验收，因为它动到既有路径）：
- 透明屏 → 液态块落入 / 空屏 → 块落入：原本靠 commands invalidate，现在多一次（幂等，无害）。
- 成本：每次「屏从停采恢复」多一次整屏 Pass A。这类翻转本身稀有（跨屏拖拽、建窗口、
  遮挡移开），且恢复帧已经在付 32 ms 阻塞采集。
- **验收**：`verify_dirty` 连跑 ≥3 次 + `verify_validity` 零虚报 + `cmp_visual` single 四段全 0。

### 2.9 配置 / 环境变量 / 快照

- `api/config.rs`：`EngineConfig` 增 `pub occlusion_culling: bool`，`Default` 为 `true`。
  值类型不加 `#[non_exhaustive]`。
- `diag/env.rs`：`EnvOverrides` 增 `pub no_occlusion: bool`（`GLASS_OCCLUSION=0`），
  与 `no_adaptive`（`GLASS_ADAPTIVE=0`）同构；模块文档注释的「例外清单」里登记一行。
- 生效判据（构造期算一次）：`let occlusion_enabled = cfg.occlusion_culling && !env.no_occlusion;`
- `api/handle.rs`：`WindowSnapshot` 增 `pub occluded: bool`；
  `publish_snapshot` 的 `snapshot_key_next` 位掩码从 `visible*2 + interactive`
  扩成 `visible*4 + interactive*2 + occluded`，否则「只有 occluded 变了」不会触发快照发布。
- `playground/src/schema.rs::liquid_fields()` **不需要**改（只约束 `LiquidParams`）。

### 2.10 日志

- 逐块翻转沿：`log::debug!`；每屏「因全部块被遮挡而停采 / 恢复」翻转沿：`log::info!`
  （与「检测到独占全屏应用，暂时隐藏玻璃」同一量级）。
- 全部走级别名，**不新增 `RAW_TARGETS`**（R-C2）；状态位 + 翻转沿，热路径零格式化（R-C3）。

---

## 3. 兼容性与回滚

| 面 | 影响 | 处置 |
|---|---|---|
| 单帧像素输出 | **逐位不变**（B1 不改绘制集合；A 只影响「什么时候画」） | AC6 用 `GLASS_OCCLUSION=0/1` 同帧对比 |
| 恢复帧 | 多一次整屏 Pass A + 全量重画（`invalidate`） | AC2 稳态帧四段全 0 |
| 公共 API | `EngineConfig.occlusion_culling` + `WindowSnapshot.occluded`（均为新增字段） | 值类型不加 `#[non_exhaustive]`；`EngineConfig` 有 `Default`，上层用 `..Default::default()` 惯例，风险低；CHANGELOG 记明 |
| cbuffer / shader | **不动** | — |
| 回滚 | B1 单点开关：`occlusion_culling=false` 或 `GLASS_OCCLUSION=0`；A 无开关（它是「隐藏画布不该干活」的缺陷修复），回滚 = revert | CHANGELOG 写明 |

## 4. 已知取舍（诚实记录，别当 bug 修）

1. **遮挡移开后 ≤125 ms 内玻璃里是旧桌面**（B1 停采的必然代价）。不影响正确性
   （恢复帧会取新桌面 + 整屏重画），只影响那一小段观感。要亚帧恢复得上 `SetWinEventHook`，
   本任务明确不做。
2. **`WS_EX_LAYERED` 窗口一律不算遮挡者**，即使 alpha=255。会漏剔一部分本该省的。
   方向保守（多渲染），符合 R-B1-4。
3. **异形窗口（`GetWindowRgn != ERROR`）一律不算遮挡者**。同上。
4. **默认 `AlwaysOnTop` 层级下 B1 收益接近 0**：`EnumWindows` 会在自家画布处几乎立刻终止，
   遮挡者集合恒空（除非别的 topmost 窗口完整覆盖某块，例如任务栏盖住贴底的小块）。
   这不是缺陷，是层级语义的必然结果 —— 置顶玻璃本来就不该被普通窗口遮住。
   真正吃到 B1 收益的是 `Normal` / `ClickToFront` / `AlwaysOnBottom`。
5. **只处理「完整覆盖」**。部分覆盖不给任何收益（Pass A 的 `pad` 外扩与脏区并集都是矩形级），
   要做部分遮挡得进 B2（改脏区集合），代价见 PRD 的 Non-Goals。
