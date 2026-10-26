# Edit Plan — P1 焦点路由与拖入命中验证

## 1. FR1 焦点路由 (set_focus_hwnd)

### 1a. Command 枚举追加 (api/handle.rs)
- 在 ClearContent 之后追加 SetFocusHwnd { id: u32, hwnd: isize }
- 不碰前面任何变体

### 1b. GlassWindow::set_focus_hwnd (api/handle.rs)
- 加在 set_input_sink 附近
- 入队 Command::SetFocusHwnd

### 1c. App 存储 (runtime/app.rs)
- 新增 lock_focus_hwnds: Vec<(u32, isize)> （仿 block_sinks 存法）
- 新增 ocus_hwnd_for(id) -> Option<isize> 查询方法
- 新增 focus_guard_warned 位（warn-once）

### 1d. 命令消费 (runtime/commands.rs)
- 新增 SetFocusHwnd arm：与 SetInputSink 同构的增删改逻辑
- 校验守卫：hwnd == canvas hwnd ⇒ 忽略 + warn-once

### 1e. 触发点 (platform/window.rs)
- RouteAction::Consume 分支内：若块有 focus_hwnd ≠ 0 ⇒ SetFocus(hwnd)
- 需要导入 SetFocus from windows crate
- 注释说明最后一道闸

### 1f. WM_KEYDOWN 注释 (platform/window.rs)
- 在 ESC→Shutdown 分支上加注释：「守卫保证画布不经库获得焦点，仅 standalone 调试可达」

## 2. FR3 验证脚本 (tools/verify_focus_drop.py)
- 新建脚本：沿确定性路径反复穿越块区域 ≥50 轮
- WindowFromPoint 多时相采样
- 稳定漏检 == 0；帧窗口 miss ≤ 2
- 焦点取证：GLASS_FOCUS_DEMO=1

## 3. FR4 README
- 新增「把 IDropTarget 挂到画布」小节

## 4. Playground
- GLASS_INPUT_SINK_DEMO 路径接 set_focus_hwnd
- 用 GetConsoleWindow() 作宿主 hwnd
- [verify] 日志报焦点结果
