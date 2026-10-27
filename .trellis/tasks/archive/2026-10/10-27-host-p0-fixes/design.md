# Design: 宿主实测四缺陷修复（对齐本地 HEAD=f2dc3d4 / 0.5.0）

来源：`research/defect-fix-plan-host.md`（宿主实测定界报告）。本文件记录
**附件补丁与本地代码的核对差异**、每条缺陷的最终设计、以及两个附件没覆盖的连带风险。

## 0. 附件 vs 本地代码的三处出入（核对结论）

| 缺陷 | 附件说法 | 本地实际 | 处理 |
|---|---|---|---|
| #1 | 新增 `App::shutting_down: AtomicBool` | `App::shutdown: bool` 已存在，且 wnd_proc 经 `DispatchMessageW` 恒在渲染线程执行（与 App 同线程）→ 无跨线程可见性问题 | 复用 `app.shutdown`，不加新字段 |
| #3 | 循环"阻塞在 GetMessage" | 主循环是 `PeekMessageW`（非阻塞）+ DDA 8/32ms 超时 + `!need_render` 8ms sleep 下限，静态推不出永久停摆 | 补丁照做（消息级兜底与宿主 PostMessage 止血同构）；Open Question 见 prd.md，用取证脚本实测钉行为 |
| #4 | 补丁 = rebuild 末尾逐块 `sync_block_monitor` | **该循环已存在**。真根因有两个：① sync 前 `b.scale = 1.0` 强清摧毁换算基准（k=new/1.0 ⇒ 物理尺寸重乘新缩放 ⇒ 200%→100% 逻辑尺寸翻倍，正是宿主"2x 膨胀"现场）；② `refresh_monitor_rects()` 在 rebuild **之后**（runtime/mod.rs L364-365），rebuild 内的归屏判定用的是旧屏矩形 | 删强清 + 提前刷新；把重估算术抽成 core 纯函数获得单测判别力 |

## 1. 缺陷 #1：WM_DESTROY 退出意图显式化（platform/window.rs）

```rust
WM_DESTROY => {
    // 重建路径（rebuild_monitors）销毁旧画布不得杀死消息循环：
    // 只有引擎整体退出（App::shutdown 已置位）才允许 PostQuitMessage。
    if app_of(hwnd).is_some_and(|app| app.shutdown) {
        PostQuitMessage(0);
    }
    LRESULT(0)
}
```

时序核对（为什么 bool 而非 AtomicBool 是安全的）：
- `app_of` 只能在渲染线程拿到有效 App（GWLP_USERDATA 由 runtime 维护）；
  `wnd_proc` 经主循环 `DispatchMessageW` 调用 ⇒ 与 App 的持有者同线程。
- rebuild 路径：先 `SetWindowLongPtrW(旧hwnd, GWLP_USERDATA, 0)` 再 `DestroyWindow`
  ⇒ `app_of` 返回 None ⇒ 不 PostQuit（双保险：既有 unbind + 新增 shutdown 判定）。
- 引擎退出路径：`Command::Shutdown` → commands 消费段 `app.shutdown = true` →
  主循环 break → `unbind_hwnds()` → `DestroyWindow` ×N → WM_DESTROY 时 USERDATA
  已清 ⇒ None ⇒ 不 PostQuit —— **没问题**：循环已经 break，run() 直接返回，
  线程退出不依赖 WM_QUIT。外部（host/OS）向线程队列投递的 WM_QUIT 仍由
  PeekMessageW 分支消费（行为不变）。
- WM_QUIT 语义不变：`PeekMessageW` 见 WM_QUIT → `app.shutdown=true` → break → 收尾。

### 连带风险 A（附件没覆盖，必查必改）：playground 主窗口的关闭路径

原行为里用户点主窗口 × → `WM_CLOSE` → DefWindowProc → `DestroyWindow` →
`WM_DESTROY` → `PostQuitMessage` 是 **standalone 模式唯一的"关窗退出"通路之一**。
去掉无条件 PostQuit 后，若主窗口 wnd_proc 不推 `Command::Shutdown`，
playground 会变成"点 × 进程不退"的僵尸。实现步骤包含：核对 `wnd_proc` 的
归属（画布与主窗口是否共用同一个过程）；主窗口路径补
`WM_CLOSE => { app.shared.push(Command::Shutdown); LRESULT(0) }`（或等价），
保持"关窗 = 引擎退出"的用户可见行为。

**核对结论（2026-10-27 planning 阶段实查）**：playground 没有自绘消息主窗口
（`main.rs` 的 `CreateWindowExW` 只在 `GLASS_FOCUS_DEMO` 退化路径造 1×1 STATIC 壳窗口，
无自定义 wnd_proc）；库画布是 `WS_POPUP` 全屏透明覆盖层，**没有可供用户点击的 ×**。
playground 退出链 = `e.shutdown()`（Ctrl+C / 面板）→ `Command::Shutdown`，
以及 `while engine.is_running()` 等待渲染线程自然结束。⇒ 去掉无条件
`PostQuitMessage` 不会产生僵尸进程路径，无需补 WM_CLOSE 分支；
AC1 脚本仍加一条终局断言（`shutdown()` 后线程正常 join、进程退出码 0）钉退出路径。

## 2. 缺陷 #2：register_class 的 HRESULT 比较（platform/window.rs）

错误本质：`Error::code()` 是 HRESULT，`HRESULT_FROM_WIN32(1410) = 0x80070582`，
与裸 `1410` 数值比较恒不等。修复取**严格相等**而非掩码（防低 16 位撞车）：

```rust
#[must_use]
pub(crate) fn is_class_already_exists(code: i32) -> bool {
    // ERROR_CLASS_ALREADY_EXISTS=1410 的 HRESULT 形态是 0x80070582，
    // 与裸 1410 做数值比较永不相等（0.5.0 前科：容忍分支注释写了、实际永不命中）。
    code as u32 == 0x8007_0582 // = HRESULT_FROM_WIN32(1410)
}
```

- 抽成纯函数 + `#[cfg(test)]` 单测钉死：`0x80070582` → true；`0x80070005`、
  裸 `1410`、`0` → false。（RegisterClassExW 失败路径的 code 恒为
  `HRESULT_FROM_WIN32(GetLastError())` 形态，严格相等覆盖真实错误来源。）
- 若确认 windows-core 0.58 暴露 `HRESULT::from_win32`，比较值用它构造、
  字面量退居注释（可读性更好；行为相同，单测口径不变）。

## 3. 缺陷 #3：画布常驻 100ms 兜底定时器（platform/window.rs）

- `create_canvas` 成功后：`SetTimer(hwnd, TIMER_ID_WAKEUP, 100, None)`；
  `TIMER_ID_WAKEUP: usize = 0x5E_55_0001`（附件的 `0xVE55_0001` 不是合法字面量，
  取一个不与库内其它 id 冲突的固定值并注释登记）。
- `wnd_proc` 新增 `WM_TIMER => { if wp.0 == TIMER_ID_WAKEUP { request_render(hwnd); }
  LRESULT(0) }`（按 wp 过滤 id；本库画布窗口不建其它定时器）。
- 为什么选 `request_render` 而不是空转返回：WM_TIMER 本身只保证"线程回到消息泵"，
  `request_render` 额外把 `requested` 置进渲染门 ⇒ 当圈必然跑完整 prep 段
  （光标轮询 + 穿透状态机 + 遮挡复检 + just_activated 恢复链）；而
  `render_monitor` 在脏区为空时走早退路径（不 Present），静止期成本≈每 100ms
  一次轻量循环。gate_perf 静止基线复跑验证（风险 B）。
- rebuild 无需额外处理：`create_canvas` 对新窗口重装；`DestroyWindow` 自动销毁定时器
  （Win32 保证）。standalone 画布同码同效。
- 已知取舍：定时器 10Hz 意味着"绝对静止"时循环每 100ms 至少带一次
  render_request 空转一圈；相对既有 8ms sleep 下限是增密而非新增唤醒类型，
  CPU 差异应落在测量噪声内（脚本+阀门双确认）。

### 连带风险 B：静止 CPU 与 idle 基线

`IDLE_WAIT_MS` 注释承诺"静止桌面 CPU 趋近 0"。修复后必须用 gate_perf /
`[metrics]` 静止段确认无阶跃回归；若有，降级方案 = WM_TIMER 不 request_render
（仅空转一圈），两方案对宿主"消息注入唤醒"路径等价。

## 4. 缺陷 #4：rebuild 重估（runtime/app.rs + core/block.rs 纯函数）

### 4.1 抽取 `rehome_block`（core 层，0-unsafe，可单测）

把 `sync_block_monitor` 的全部算术（中心归屏 → 触发条件 → 新旧 scale 换算 →
中心不出屏钳制 → Block::clamp 同款尺寸夹）移植为纯函数：

```rust
pub(crate) struct MonitorGeom { pub origin: [i32; 2], pub size: [i32; 2], pub scale: f32 }
pub(crate) struct BlockGeom { pub x: f32, pub y: f32, pub width: f32, pub height: f32,
                              pub scale: f32, pub monitor: usize }
pub(crate) fn rehome_block(g: BlockGeom, monitors: &[MonitorGeom]) -> BlockGeom
```

- `sync_block_monitor` 变薄壳：读字段 → 构造入参（scale 侧保持既有 `max(0.05)`
  口径与 `old_scale.max(1e-3)` 除零保护，逐位等价）→ 调 rehome → 写回。
  命令路径（块几何命令/拖拽跨屏）行为不变。
- 归屏查找算法 `display::index_at_rects`（纯函数）下沉 `core/geometry.rs`
  （core 不得依赖 platform），`platform/display.rs` 原入口改薄转发，保持单一来源。
- 尺寸夹 `Block::clamp` 的算术抽出为 `core/block.rs::clamp_geom`（Block::clamp 与
  rehome_block 共用；NaN 语义保持 clamp_finite 纪律）。

### 4.2 rebuild_monitors 的两处修改

```rust
app.monitors = next;
app.refresh_monitor_rects();          // ← 新增：必须在块重估之前（修根因 ②）
for hwnd in old { ...DestroyWindow... }
app.style_cache.clear();
let ids: Vec<u32> = ...;
for id in ids {
    // 删除 b.monitor=0 / b.scale=1.0 强清（修根因 ①）：
    // b.scale 保留值就是"块当前呈现所在屏的 scale"，正是新旧换算的基准；
    // 归屏由 rehome_block 按中心重估，monitor 旧值只影响触发条件（不等则换算）。
    sync_block_monitor(app, id);
}
```

- runtime/mod.rs L364 的外层 `refresh_monitor_rects()` 保留（幂等、便宜，
  作为"任何 rebuild 入口都兜住"的第二道）。
- `app.snapshot_dirty = true` 已有。

### 4.3 单测矩阵（core/block.rs）

1. `same_screen_same_scale_is_noop`（逐位不变）。
2. `down_200_to_100_keeps_logical_size`：1000px@2.0 → 500px@1.0；
   **回归钉**：老行为（强清 1.0）算出 1000px = 2x 膨胀，测试注释写实测前科。
3. `up_100_to_200_keeps_logical_size`。
4. `monitor_index_shrink_stays_valid`：旧 monitor=5、新只剩 2 屏 → 按中心重归，索引 <2。
5. `offscreen_center_pulled_back`：分辨率缩小使中心出屏 → 拉回合法区。
6. `primary_switch_negative_origin`：左副屏 origin 为负时的归屏。

## 5. 取证脚本（tools/，family 风格参照 verify_input_sink / verify_occlusion）

### `verify_monitors_rebuild.py`（AC1 + AC4 行为面）
- 启 playground（hermetic env 家族口径），按类名找画布 HWND 集合。
- `PostMessageW(H1, WM_DISPLAYCHANGE)` → 等"显示器集合变化，画布与采集已重建"日志。
- 断言：进程存活；重建后 `[metrics]`/渲染仍在推进（帧号增长 ≥N）；无 Shutdown；
  旧 HWND 已 Destroy、新 HWND ≠ 旧（证明确实走了重建）；重建日志后 2s 内无
  `RegisterClassExW 失败`（顺带钉 #2 在重建路径的回归）。
- 判别力：未修复时线程必死于重建 ⇒ 帧推进断言必红；"删实现还红吗"成立。

### `verify_clickthrough_recovery.py`（AC3）
- 读块矩形（`[verify]` state 行，家族手法）→ 光标物理停放到块外远处。
- ctypes 造不透明 TOPMOST 覆盖窗盖块 → 等「桌面采集停产」日志（≤3s，含迟滞）。
- 销毁覆盖窗（等价"移开"）→ 等恢复；再 `SetCursorPos` 进块中心 →
  轮询 `GetWindowLongW(canvas, GWL_EXSTYLE) & WS_EX_TRANSPARENT == 0`，预算 300ms。
- 反向对照（防假绿）：光标移出屏外远处后再轮询 exstyle 必须**回到**穿透置位。
- 终局断言：mouse_event 物理按下拖动块 → 块矩形变化（交互真实恢复）。

两脚本均 `VERRE_EXE` 可覆盖、`GLASS_DEFAULT_BLOCK=1`，不进 CI（依赖真实桌面）。

## 6. 影响面清单

- `platform/window.rs`：WM_DESTROY 分支、`is_class_already_exists`、create_canvas 装
  定时器、WM_TIMER 分支、（若需要）主窗关闭路径。
- `runtime/app.rs`：`rebuild_monitors`（refresh 提前、删强清）、`sync_block_monitor` 薄壳化。
- `core/block.rs`：`rehome_block` + `clamp_geom` + 单测。
- `core/geometry.rs`：`index_at`（自 platform/display.rs 下沉）。
- `platform/display.rs`：`index_at_rects` 转发。
- `tools/`：新增 2 脚本。
- `CHANGELOG.md`：新增 `## [0.5.1-unreleased]` Fixed 四条。
- **零改动**：api/、shaders、cbuffer、Command 枚举、EngineEvent、schema.rs、BASELINE 不变量。

## 7. 验证与回滚

- 修复前先跑一遍取证脚本记录基线现象（repro 记录进 research/repro-baseline.md，
  复现与否都是结论，必须留痕）。
- 修复后：五连全绿 + 复跑 `verify_dirty`（≥3 连）/ `verify_mouse_isolation` /
  `verify_validity` / `repro_tremble far|many` / `verify_adaptive` / `verify_occlusion` /
  `verify_crossscreen` / `verify_input_sink` / `verify_content` / `verify_focus_drop` /
  `ab_latency` + gate_perf（性能取证期间不并发构建，项目纪律）。
- 回滚点：master 当前 HEAD（f2dc3d4）即回滚基线；本任务改动集中、可单 commit revert。

## 8. 复现阶段增补（2026-10-27 基线实测，详见 research/repro-baseline.md）

### 8.1 新缺陷 #5：`GLASS_MONITORS` truncate 泄漏孤儿画布（纳入本任务修复）

基线实锤：`GLASS_MONITORS=1` 启动后枚举到 **2** 块画布 —— `runtime/mod.rs` boot 的
`monitors_rt.truncate(n)` 只丢 `MonitorRt` 结构，**被丢显示器的 HWND 未 DestroyWindow**。
孤儿窗口 USERDATA 未绑定（恒穿透、不参与渲染、占 topmost z 序带），还会吞掉发给它的
消息（复现脚本第一版被它坑掉）。修复：truncate 前对将被丢弃的 `m.hwnd` 逐个
`DestroyWindow`（此时尚未 bind_hwnds，无悬垂指针风险；采集/合成器随 MonitorRt drop
自然释放 —— 需确认 drop 不依赖窗口存活；若有顺序要求则先释放 capture 再毁窗，
以 `MonitorRt` 字段声明顺序为准）。

### 8.2 新缺陷 #5b：rebuild_monitors 不应用 monitor_limit（口径不对称，纳入修复）

基线实锤：刺激后画布 1 -> 3（rebuild 重枚举全部屏）。修复：`App` 新增
`monitor_limit: Option<usize>`（构造期从 `env.monitor_limit.or(cfg.monitor_limit)`
读一次，符合「环境变量只在构造期读」纪律），`rebuild_monitors` 在 enumerate 后
同样截断（truncate 前先建后毁的顺序沿用现有 old/new 流程，无孤儿风险）。
修复后 `verify_monitors_rebuild.py` 的 C1/C3 口径自动变干净（刺激前后各 1 块）。

### 8.3 缺陷 #3 本地不复现的结论与补丁定位（Open Question 已回答）

基线（无定时器）上 clickthrough 脚本 6/6 绿、A3=21ms。本地循环无停摆路径；
宿主自锁是其环境特有。补丁仍照做（design §3），定位从「根因修复」调整为
「**库侧自包含兜底驱动源**」：等效把宿主已验证有效的 wake_canvas_idle 内建化，
不再依赖宿主线程外部注入。脚本作为常驻回归位保留：未来若本地出现停摆路径，
它必须先在 300ms 预算处红。

### 8.4 缺陷 #2 的验证载体定案

rebuild 路径不经过 register_class（boot-only），故 #2 的复现载体 = 同进程两轮
`Engine::new + start + shutdown`。落为 `tests/api_surface.rs` 的 `#[ignore]` 集成测试
（真实桌面才可跑，`cargo test --release -p liquid-verre-rs -- --ignored` 手工/脚本
取证执行；CI 的 test --workspace 默认跳过 ignore，不违反「桌面取证不进 CI」红线）。
外加 `is_class_already_exists` 纯函数单测钉比较逻辑本体。

### 8.5 口径登记（不修）：`WindowSnapshot.rect` 文档说「逻辑像素」，实测输出是

**物理像素**（默认块 (960,660,1200,760) = 3120x2080 居中物理值；scale 字段=1.0）。
属文档/口径漂移，与附件 #4 的「撕裂」可能有历史牵连，但改动面涉及宿主契约解读 ——
本任务不动，登记给后续任务评估（若宿主按逻辑像素消费 rect，那才是撕裂的另一半根因）。

