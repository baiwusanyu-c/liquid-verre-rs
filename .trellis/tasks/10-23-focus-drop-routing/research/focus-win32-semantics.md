# Research — 跨线程键盘焦点：Win32 语义与本库策略（2026-10-26 主会话裁决）

## 事实链（实验实锤，非推断）

1. `SetFocus` 只作用于**调用线程创建**的窗口；跨线程/跨进程直接调用返回 NULL。
   证据：库侧日志 `focus_set block=1 target=0x1b07da ret=0x0`（渲染线程调宿主窗口，恒失败）。
2. 画布是 `WS_EX_NOACTIVATE`（故意的：抬层用 SWP_NOACTIVATE，不许抢前台）⇒ 用户按压画布
   **不会**把前台切到宿主进程。
3. 因此「库代宿主 SetFocus」在不 attach 输入队列的前提下**不可能兑现**。

## 方案对比

| 方案 | 机制 | 风险 | 裁决 |
|---|---|---|---|
| A. 裸 SetFocus（P1 初版） | 渲染线程直接 SetFocus(宿主 hwnd) | **恒失败**（ret=0），API 变成摆设 | 驳回 |
| B. AttachThreadInput 桥 | render ↔ 目标线程短暂 attach 后 SetFocus | attach 滞留 = 宿主线程卡死拖死渲染线程（输入锁同生死）；非前台时仍可能被系统策略拒 | **采纳为 best-effort**：attach 窗口微秒级、失败不重试、warn-once、连击 100 次实测无卡顿才算过 |
| C. 宿主自聚焦（兜底，永远可用） | sink Down=Consumed 回调里宿主 **PostMessage 给自己的壳窗口**（PostMessage 任意线程合法、无锁），壳窗口在自己线程 SetFocus 自己 | 无库侧风险；语义归位（焦点是宿主的） | **采纳为文档主推模式** |

结论：`set_focus_hwnd` 保留但契约降级为 best-effort（B），README/rustdoc 主推 C 并两种都给示例；
ikun 清单 P1-1「使键盘可达」由 C 即可完全满足，B 是锦上添花。守卫（拒绝画布自身 hwnd）
保留——它保证 ESC→Shutdown 死路径不会因误用而活。

## 教训（进 spec 候选）

- 「API 有日志 ≠ 功能生效」：取证字段必须断言**返回值**，不是调用发生过。
- 跨线程 GUI 调用的三种合法姿势只有：同线程、AttachThreadInput、PostMessage 回目标线程。
  写 Windows 集成层 API 时先问：调用发生在谁的线程？
