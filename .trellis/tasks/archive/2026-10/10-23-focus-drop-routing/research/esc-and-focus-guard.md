# Research — ESC 死路径处置与 set_focus_hwnd 防误设计（P1 开放问题关闭）

主会话分析（2026-10-26 11:5x，代码现场为 Erdos 返工期间，行号或有漂移、结论不受影响）。

## 事实

1. `platform/window.rs` 现存 `WM_KEYDOWN` 分支：`ESC => push(Command::Shutdown)`，其余键 `request_render + LRESULT(0)`（吞键）。
2. 全仓 `rg SetFocus` = **0 命中**：库今天从不给任何窗口设焦点，画布 `WS_EX_NOACTIVATE` ⇒ 键盘消息在现架构里**不可达**画布（这条分支是死路径，注释自认）。
3. P1-1 `set_focus_hwnd` 落地后，将首次出现「库自己调 SetFocus」的代码路径 ⇒ 如果宿主把**画布自己的 HWND** 传进来（误用），画布就可能真的收到键盘：ESC 会**关停整个引擎**，其他键被画布吞掉（宿主 UI 失联）。

## 决策（推荐，供 P1 实施）

**保留 ESC 分支 + 在 set_focus_hwnd 入口加防呆守卫**，理由：

- 守卫：`set_focus_hwnd(hwnd)` 实现里 `if hwnd == 本画布 HWND { log::warn!(once) ; 忽略命令 }`。
  画布永不经库获得焦点 ⇒ ESC 分支保持「只能被刻意绕过库（Win32 API 直调）才可达」——
  那种场景就是 standalone 调试，ESC 关停反而是想要的逃生门（与 0.4.2 行为一致性也不破：
  分支不删，sink-less/无焦点构建的行为逐字节不变，cmp_visual 零差异原则无额外风险）。
- 拒绝的备选 A（删分支）：丢调试逃生门，且属于对 0.4.2 可见行为的删除——最小改动原则不过关。
- 拒绝的备选 B（env 门控）：为一个死代码新增库级 env（要进 diag/env.rs 例外清单 + 文档 + 测试），
  收益配不上成本；守卫一行就能达意。
- 其余键吞掉（不 DefWindowProc）：维持现状。误配焦点时「吞键」比「DefWindowProc 弹错误音」伤害小；
  有了守卫后此场景不可达，不值得设计更多。

## P1-1 实施要点补充（focus-drop-routing）

- `Command::SetFocusHwnd { id: u32, hwnd: isize }`（尾部追加）；存储随块；触发时机见父 design §3。
- 守卫判定的「本画布 HWND」：渲染线程帧首消费时可直接对比 per-monitor canvas hwnd（runtime 持有权在平台点位内）。
- hwnd==0 ⇒ 清除（等价未设置）；无效 hwnd 的 SetFocus 失败静默（Win32 返回 NULL），warn-once 状态位即可，不查询有效性。
- 单测：命令入队矩阵（api 侧）；守卫分支属 platform/runtime 点位，由 playground demo + GetGUIThreadInfo 取证覆盖（AC2）。
