# liquid-verre-rs 0.5.0 缺陷修复方案（宿主 ikun 分类框实测定界，可直接照改）

> 基线：crates.io `liquid-verre-rs 0.5.0`。四条缺陷全部在宿主真实环境复现并定位到源码行。
> 每条含：现象 → 复现 → 根因 → **补丁（before/after）** → 验证。
> 宿主侧对应止血：`wake_canvas_idle`（#3）、`revive_if_dead + UnregisterClassW`（#1/#2）。
> 库修复合并发版后，宿主止血降级为保险丝。

---

## 缺陷 #1（P0）：显示器变化重建画布时，渲染线程被自己杀死

**现象**（宿主日志，2026-10-27 09:12:50）：
```
INFO  liquid_verre::runtime   显示器集合变化，画布与采集已重建，显示器数量: 2
INFO  glass_engine            玻璃引擎已退出          ← 紧跟着就 Shutdown 了
```
用户热插拔/改缩放后，全部玻璃消失，宿主退回旧渲染。

**根因**：`platform/window.rs` 画布 wnd_proc：
```rust
WM_DESTROY => {
    PostQuitMessage(0);   // ← 无条件杀消息循环
    LRESULT(0)
}
```
而 `runtime/app.rs` 的 `rebuild_monitors()`（L491 附近）对旧画布 `DestroyWindow` →
`WM_DESTROY` → `PostQuitMessage(0)` → 渲染线程的 `GetMessage` 返回 0 → `runtime::run`
主循环退出 → 线程结束并 emit `EngineEvent::Shutdown`。**画布销毁 ≠ 引擎退出**，但被
混用了同一条退出路径。

**补丁**：把「退出意图」显式化，画布销毁只在引擎真正 shutdown 时 PostQuit。

`runtime/app.rs`（App 结构体）：
```rust
pub(crate) struct App {
    // ...
    /// 引擎正在整体退出（只有此时画布 WM_DESTROY 才允许 PostQuitMessage）。
    pub(crate) shutting_down: std::sync::atomic::AtomicBool,
}
```
`platform/window.rs`：
```rust
WM_DESTROY => {
    // 重建路径（rebuild_monitors）销毁旧画布不得杀死消息循环；
    // 只有引擎退出路径（runtime::run 收尾统一 PostQuit）允许终止循环。
    if let Some(app) = app_of(hwnd) {
        if app.shutting_down.load(std::sync::atomic::Ordering::Relaxed) {
            PostQuitMessage(0);
        }
    }
    LRESULT(0)
}
```
`runtime/mod.rs`（`run()` 的退出收尾，处理 `Command::Shutdown` 后）：
```rust
// 消费到 Shutdown：置标志 → 销毁全部画布 → 由本线程自己 PostQuit 收尾
app.shutting_down.store(true, std::sync::atomic::Ordering::Relaxed);
destroy_all_canvases(&mut app);
windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
```

**验证**：`ChangeDisplaySettingsEx` 改主屏分辨率（或拔插外接屏）→ 断言引擎继续存活：
后续帧日志/`Engine::is_running()==true`，块仍正常渲染。

---

## 缺陷 #2（P0）：register_class 的「类已存在」容忍分支永远不命中

**现象**（宿主日志 09:13:21，紧随 #1 之后）：
```
ERROR liquid_verre::api::engine  引擎异常退出: [window-creation] RegisterClassExW 失败: 类已存在。 (0x80070582)
```
同进程重建第二个 Engine 必然失败 → 玻璃永久丢失。代码注释明确写了「已经注册过时返回
成功」，但比较写错了。

**根因**：`platform/window.rs` L79-88：
```rust
let err = windows::core::Error::from_win32();
// ERROR_CLASS_ALREADY_EXISTS = 1410
if err.code().0 as u32 != 1410 { return Err(...); }
```
`Error::code()` 是 **HRESULT**：`HRESULT_FROM_WIN32(1410) = 0x80070582`，与 `1410`
做数值比较永不相等 → 本应容忍的路径反而 `Err`。

**补丁**：
```rust
// after：低 16 位才是 Win32 错误码
if err.code().0 as u32 & 0xFFFF != 1410 {
    return Err(Error::with_source(ErrorKind::WindowCreation, "RegisterClassExW 失败", err));
}
```

**验证**：同进程 `Engine::new` + `start` + `shutdown` × 2 轮，第二轮不得报 window-creation。
（宿主 `revive_if_dead` 已实测：UnregisterClass 绕开后重建即成功，坐实此根因。）

---

## 缺陷 #3（P0，「分类框全部交互死亡」的主因）：采集停产 → 主循环停摆 → 点击穿透永不解除

**现象**（宿主实测，2026-10-27 11:0x）：
1. 用不透明窗口（Chrome）盖住玻璃块 → 库日志 `全部玻璃块被不透明窗口完整遮挡，桌面采集停产`；
2. 移开窗口后，块可见，但**所有鼠标交互永久失效**：光标进块 900ms 后画布 exstyle 仍带
   `WS_EX_TRANSPARENT`（宿主 ctypes/Win32 直读实测），宿主 sink 零事件；
3. 不会自愈，重启进程才恢复。独占全屏让路（`fullscreen_hide`）路径同理。

**根因**：「光标在块内 → 解除穿透」的判定在**主循环体内**（`runtime/mod.rs` L770-783）：
```rust
let inside = mon_dragging || mon.blocks.iter().any(|blk| { blk.interactive && ... });
window::set_click_through(mon.hwnd, inside);
```
主循环的唤醒源 = 采集帧到达 / 渲染请求 / 画布窗口消息。遮挡停产后采集不再驱动循环；
画布一旦处于穿透态，**鼠标消息也不再进画布** → 循环阻塞在 `GetMessage` → 这段判定
（含光标 `GetCursorPos` 轮询）永远不执行 → 穿透态自锁。`idle_keepalive_ms` 只保活
「采集」，不覆盖该状态机。

**补丁（最小改动）**：给画布常驻一个 100ms 兜底定时器，保证循环每 100ms 至少空转一圈。

`platform/window.rs`（`create_canvas` 成功之后）：
```rust
// 采集停产/遮挡期间主循环无帧时钟；定时器保证「光标轮询 + 穿透状态机 +
// 遮挡复检」每 100ms 仍被驱动一次。WM_TIMER 只在队列空闲时产生，零成本兜底。
const TIMER_ID_WAKEUP: usize = 0xVE55_0001; // 任选不与库内其它 id 冲突的值
let _ = SetTimer(mon.hwnd, TIMER_ID_WAKEUP, 100, None);
```
`platform/window.rs`（画布 wnd_proc）：
```rust
WM_TIMER => {
    request_render(hwnd);   // 或仅 return LRESULT(0)：醒来即空转一圈，状态机自行推进
    LRESULT(0)
}
```
重建路径无需额外处理：`create_canvas` 重装定时器，`DestroyWindow` 自动销毁。

（等价替代：主循环把 `GetMessage` 换成 `MsgWaitForMultipleObjectsEx(0, .., 100ms, QS_ALLINPUT)`。）

**验证**：复现脚本化——不透明窗口盖块 → 等「采集停产」日志 → 移开 → `SetCursorPos` 进块 →
断言 300ms 内 `GetWindowLongW(canvas, GWL_EXSTYLE) & WS_EX_TRANSPARENT == 0`。
（宿主 `wake_canvas_idle` 的 PostMessage 唤醒已实测：唤醒后穿透立即解除、sink/按钮点击恢复，
反证根因在循环停摆。）

---

## 缺陷 #4（P1）：MonitorsChanged 后块几何不按新 DPI 重估，块滞留旧坐标空间

**现象**：200% ↔ 100% 切换（或主屏变更）后，玻璃块画在屏外/半屏错位，与宿主内容窗口
撕裂；直到该块下一次 `set_rect/set_position`（触发 `sync_block_monitor`）才自愈。
宿主上午的「块尺寸 2x 膨胀」现场即此问题的表现之一。

**根因**：`runtime/app.rs::rebuild_monitors()` 重建 monitors/画布/采集，但 `Manager`
中各块的 `b.x/b.y/b.width/b.height/b.scale/b.monitor` 原样保留——`scale_of` 的新旧比例
换算只存在于 `sync_block_monitor`，而它只在块几何命令时被调用。

**补丁**：`rebuild_monitors()` 完成新 monitors 后，对全部块补一次重估：
```rust
// 重建 monitors 完成后：逐块重估所在屏与 DPI 比例（保持逻辑尺寸不变），
// 并把因分辨率变化而越界的块拉回「中心不出屏」的合法区。
let ids: Vec<u32> = app.manager.blocks().iter().map(|b| b.id).collect();
for id in ids {
    sync_block_monitor(&mut app, id);
}
app.snapshot_dirty = true;
```

**验证**：改主屏缩放 200%→100% → 断言块的 `WindowSnapshot.rect` 立即按新 scale 重估
（宿主侧断言块矩形与内容窗口矩形一致，无撕裂）。

---

## 宿主侧止血与库修复的对应关系（库发版后可清理）

| 库修复 | 宿主止血（可移除项） |
|---|---|
| #1 + #2 | `glass_engine::revive_if_dead` 里的 `unregister_canvas_class()` 可移除；`revive_if_dead` 本身建议保留（断电/驱动重置等仍可能杀死线程，自动复活是产品级健壮性） |
| #3 | `platform_runtime::tick` 里的 `wake_canvas_idle()` 可移除（建议保留一个低频版本作为保险丝） |
| #4 | 宿主 `attach/rebuild` 的 ratio 重算逻辑保留（与本修复互补） |

## 优先级建议
先修 #3（交互全灭，用户感知最强）→ #1+#2（热插拔后永久降级，成对出现）→ #4（撕裂/错位）。
