# PRD — P1 焦点路由与拖入命中验证（轻量任务，PRD-only）

> 前置：父任务 design.md §3。依赖子任务 `10-23-input-sink` 完成（P1-1 触发点在 Down/Consumed 路径上）。
> 实施细节沿用父 design §3，本文件即全部范围。

## 需求

- FR1 `GlassWindow::set_focus_hwnd(&self, hwnd: isize)`（isize=HWND 形态，与 `canvas_handle()` 风格一致）。
  触发点：① 块按压 Down 且 sink 返回 `Consumed`（该按压序列开始时）；② 块被点击抬层时（如实现有
  抬层路径）。画布保持 `WS_EX_NOACTIVATE`，抬层继续 `SWP_NOACTIVATE`（platform/level.rs 纪律不动）。
  SetFocus 在渲染线程 wnd_proc 上下文执行；hwnd=0/无效 ⇒ 忽略 + warn-once。
- FR2 键盘消息由宿主壳窗口自行接收，库不转发键盘。**处置开放问题**：`window.rs` 现存
  ESC→`Command::Shutdown` 分支（236 行）历史上因 NOACTIVATE 拿不到焦点而是死路径；P1 之后宿主误把
  焦点给画布时它会「活」起来并关停引擎。本任务评估并处理：倾向仅 playground/standalone 生效
  （env 或构造参数门控），库默认不响应画布键盘——结论写进 design 变更记录。
- FR3 拖入命中验证（对宿主清单 P1-2 的兑现）：实测 `WS_EX_TRANSPARENT` 逐帧切换下，
  `WindowFromPoint`/OLE 拖入探测在块区域内的命中稳定性。取证：脚本模拟拖拽路径经过块 ≥50 次遍历，
  断言画布命中零丢帧窗口；若存在竞态，采用父 design §3 降级预案（块存在期间常关 TRANSPARENT，
  仅靠 NCHITTEST 穿透），并重跑 `verify_mouse_isolation` + `verify_dirty` ≥3。
- FR4 README 新增「把 IDropTarget 挂到画布」章节：命中/pump 稳定性承诺（含降级后的真实契约）、
  屏幕坐标↔块内坐标换算说明、与 sink 契约（禁 DoDragDrop 于回调内；壳窗口锚点模式）交叉引用。

## 验收标准

- AC1 无视觉改动（焦点/文档为主）：8 回归脚本 + verify_dirty ≥3 全绿即可（gate_perf 抽查一轮）。
- AC2 demo：playground 壳窗口在 sink=Consumed 按压后获得焦点（`GetGUIThreadInfo`/日志取证），
  宿主端 ESC 不再触发引擎关停（FR2 处置后的行为验证）。
- AC3 拖入命中取证数据（AC FR3）留存任务目录；若走降级预案，附像素/鼠标隔离回归结果。
- AC4 rustdoc + README + CHANGELOG。
