//! Win32 窗口层：窗口类 / 画布创建 / 窗口过程 / 命中测试 / 窗口区域 / 可见性。
//!
//! 这一层承载了三条实测得来的硬约束，改之前务必读注释：
//! 1. **拖拽期零 `SetWindowRgn`**：按下时一次性把区域扩到整窗，抬起后收紧。
//!    旧实现每帧重建 HRGN + SetWindowRgn，会让 DWM 反复重裁剪整个 overlay，
//!    是 100~280 ms 主循环长尾的主要来源。
//! 2. **空区域必须真的空**：`SetWindowRgn(hwnd, NULL)` 会恢复整窗矩形、把桌面重新挡死
//!    （`WS_EX_TRANSPARENT` 不是命中测试契约），因此无块时要设一个 0×0 的 HRGN。
//! 3. **命中测试用 `WM_NCHITTEST` 返回 `HTTRANSPARENT`**，配合逐帧切换 `WS_EX_TRANSPARENT`，
//!    做到「块内可交互、块外完全穿透」。

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, GDI_REGION_TYPE, RGN_OR,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::api::config::WindowLevel;
use crate::api::event::EngineEvent;
use crate::api::handle::Command;
use crate::api::input::{PointerButton, PointerEvent, PointerPhase};
use crate::core::handle::ResizeEdges;
use crate::core::routing::{self, PhaseKind, RouteAction, RouteInput, RouteState, SinkDecision};
use crate::error::{Error, ErrorKind, Result};
use crate::input::{DragSession, MoveState, ResizeState};
use crate::platform::{display::MonitorInfo, level};
use crate::runtime::{app::App, monitor::MonitorRt};

/// 画布窗口类名（窄字符）。
pub(crate) const CANVAS_CLASS_NAME: &str = "LiquidVerreCanvas";

/// 画布窗口类名（宽字符，含结尾 0）。
///
/// 由 [`CANVAS_CLASS_NAME`] 在**编译期**展开，保证「注册用的名字」与
/// `is_own_window` 比较用的名字是同一个来源，不会漂移。
const CANVAS_CLASS_NAME_W: &[u16] = &{
    let bytes = CANVAS_CLASS_NAME.as_bytes();
    let mut buf = [0u16; CANVAS_CLASS_NAME.len() + 1];
    let mut i = 0;
    while i < bytes.len() {
        buf[i] = bytes[i] as u16;
        i += 1;
    }
    buf
};

/// 兜底唤醒定时器 ID（100 ms）。
///
/// 唯一性登记：本库画布窗口只有这一个定时器，不与其它 id 冲突
/// （WM_TIMER 分支按 id 过滤，wp 不是它就不动作）。
const TIMER_ID_WAKEUP: usize = 0x5E55_0001;

/// 类名的 `PCWSTR` 形式。
#[must_use]
pub(crate) fn class_name_w() -> PCWSTR {
    PCWSTR(CANVAS_CLASS_NAME_W.as_ptr())
}

/// 取模块句柄。
#[must_use]
pub(crate) fn module_instance() -> HINSTANCE {
    let hmod = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
    HINSTANCE(hmod.0)
}

/// 注册画布窗口类（进程内一次）。
///
/// 已经注册过（同进程第二个 `Engine`）时返回成功而不是报错 —— 类是进程级资源，
/// 两个引擎共用同一个类完全没问题。
pub(crate) fn register_class(hinst: HINSTANCE) -> Result<PCWSTR> {
    let name = class_name_w();
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_CLASSDC,
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinst,
        lpszClassName: name,
        ..Default::default()
    };
    if unsafe { RegisterClassExW(&wc) } == 0 {
        let err = windows::core::Error::from_win32();
        // 注意：err.code() 是 HRESULT —— ERROR_CLASS_ALREADY_EXISTS(1410) 的
        // HRESULT 形态是 0x80070582，与裸 1410 做数值比较永不相等（0.5.0 前科，
        // 判据本体见 is_class_already_exists + 单测）。
        if !is_class_already_exists(err.code().0) {
            return Err(Error::with_source(
                ErrorKind::WindowCreation,
                "RegisterClassExW 失败",
                err,
            ));
        }
    }
    Ok(name)
}

/// 判断 HRESULT 是否对应「窗口类已存在」（ERROR_CLASS_ALREADY_EXISTS=1410）。
///
/// HRESULT_FROM_WIN32(1410) = 0x80070582。与裸 1410 做数值比较永不相等
/// —— 这是 0.5.0 的 #2 根因：Error::code() 返回 HRESULT，不是 Win32 错误码。
/// 此处取**严格相等**，不用低 16 位掩码（防撞车，如 0x80070005 的低 16 位巧合）。
#[must_use]
pub(crate) fn is_class_already_exists(code: i32) -> bool {
    // windows-core 0.58 的 HRESULT::from_win32(1410).0 = 0x80070582
    code as u32 == 0x8007_0582
}

/// 创建一块显示器的全屏画布窗口。
pub(crate) fn create_canvas(
    _gfx: &crate::gfx::Device,
    info: &MonitorInfo,
    lvl: WindowLevel,
) -> Result<HWND> {
    let hinst = module_instance();
    let class_name = class_name_w();
    // 初始扩展样式按层级决定要不要 TOPMOST；点击穿透由主循环逐帧维护。
    let mut ex =
        WS_EX_NOREDIRECTIONBITMAP | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
    if lvl.is_topmost() {
        ex |= WS_EX_TOPMOST;
    }
    let hwnd = unsafe {
        CreateWindowExW(
            ex,
            class_name,
            windows::core::w!("liquid verre canvas"),
            WS_POPUP,
            info.origin[0],
            info.origin[1],
            info.size[0],
            info.size[1],
            HWND::default(),
            None,
            hinst,
            None,
        )
    }
    .map_err(|e| Error::with_source(ErrorKind::WindowCreation, "CreateWindowExW(画布) 失败", e))?;
    if hwnd.0.is_null() {
        return Err(Error::new(
            ErrorKind::WindowCreation,
            "CreateWindowExW 返回空 HWND",
        ));
    }
    unsafe {
        // 让 DComp 接管整窗（无重定向位图）
        let m = MARGINS {
            cxLeftWidth: -1,
            cxRightWidth: -1,
            cyTopHeight: -1,
            cyBottomHeight: -1,
        };
        let _ = DwmExtendFrameIntoClientArea(hwnd, &m);
        // 自反馈根治：把该画布从所有采集（DDA / WGC / PrintWindow）中剔除。
        const WDA_EXCLUDEFROMCAPTURE: u32 = 0x0000_0011;
        if let Err(e) =
            SetWindowDisplayAffinity(hwnd, WINDOW_DISPLAY_AFFINITY(WDA_EXCLUDEFROMCAPTURE))
        {
            log::warn!("SetWindowDisplayAffinity({}): {e:?}", info.name);
        }
    }
    // 兜底定时器：采集停产 / 遮挡期间没有帧时钟，穿透状态机与遮挡复检
    // 不得依赖鼠标消息自身唤醒。100 ms 分辨率足够：不追求帧级响应，
    // 只保证「停产恢复后 100 ms 内巡检到」。
    unsafe {
        let _ = SetTimer(hwnd, TIMER_ID_WAKEUP, 100, None);
    }
    Ok(hwnd)
}

/// 对全部画布应用层级（需求 F4）。
pub(crate) fn apply_level(monitors: &[MonitorRt], lvl: WindowLevel) {
    for mon in monitors {
        level::apply(
            mon.hwnd,
            lvl,
            mon.ready && !mon.hidden_fs && !mon.hidden_capture && !mon.hidden_empty,
        );
    }
}

/// 取与窗口关联的 `App`；未设置时返回 `None`。
///
/// # Safety
/// `hwnd` 的 `GWLP_USERDATA` 必须要么为 0，要么指向一个存活的 `App`。
/// 这个不变量由 `runtime` 在创建 / 重建 / 销毁画布时维护（销毁前先清零）。
pub(crate) unsafe fn app_of(hwnd: HWND) -> Option<&'static mut App> {
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if raw == 0 {
        None
    } else {
        Some(&mut *(raw as *mut App))
    }
}
extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_LBUTTONDOWN => {
                dispatch_pointer(hwnd, PointerPhase::Down, Some(PointerButton::Left), 0);
                LRESULT(0)
            }
            WM_LBUTTONDBLCLK => {
                dispatch_pointer(hwnd, PointerPhase::DoubleClick, None, 0);
                LRESULT(0)
            }
            WM_RBUTTONDOWN => {
                dispatch_pointer(hwnd, PointerPhase::RightDown, Some(PointerButton::Right), 0);
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                dispatch_pointer(
                    hwnd,
                    PointerPhase::Up(Some(PointerButton::Right)),
                    Some(PointerButton::Right),
                    0,
                );
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) as i16) as i32 / 120;
                dispatch_pointer(hwnd, PointerPhase::Wheel, None, delta);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                if let Some(app) = app_of(hwnd) {
                    app.cursor = app.read_cursor();
                    forward_move_to_sink(app);
                }
                LRESULT(0)
            }
            WM_NCHITTEST => {
                let Some(app) = app_of(hwnd) else {
                    return LRESULT(HTTRANSPARENT as isize);
                };
                let c = app.read_cursor();
                let pressing = matches!(app.route_state, RouteState::Pressing(_));
                let hit = app.drag.is_some()
                    || pressing
                    || app.manager.hit_test(c[0], c[1]).is_some()
                    || app.manager.hit_resize(c[0], c[1]).is_some();
                LRESULT(if hit {
                    HTCLIENT as isize
                } else {
                    HTTRANSPARENT as isize
                })
            }
            WM_SETCURSOR => {
                let hit_test_code = (lp.0 as u32) & 0xffff;
                if hit_test_code == HTCLIENT {
                    if let Some(app) = app_of(hwnd) {
                        if let Some(id) = app.hover_handle.cursor_id() {
                            let cursor = LoadCursorW(None, PCWSTR(id as usize as *const u16))
                                .unwrap_or_default();
                            if !cursor.0.is_null() {
                                SetCursor(cursor);
                                return LRESULT(1);
                            }
                        }
                        let c = app.read_cursor();
                        if let Some(hit_id) = app.manager.hit_test(c[0], c[1]) {
                            if let Some(sink) = app.sink_for(hit_id) {
                                if let Some(cursor_id) = sink.cursor_override(hit_id) {
                                    let cursor =
                                        LoadCursorW(None, PCWSTR(cursor_id as usize as *const u16))
                                            .unwrap_or_default();
                                    if !cursor.0.is_null() {
                                        SetCursor(cursor);
                                        return LRESULT(1);
                                    }
                                }
                            }
                        }
                    }
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_LBUTTONUP => {
                dispatch_pointer(hwnd, PointerPhase::Up(None), Some(PointerButton::Left), 0);
                LRESULT(0)
            }
            WM_TIMER => {
                // 只响应我们的兜底唤醒定时器
                if wp.0 == TIMER_ID_WAKEUP {
                    request_render(hwnd);
                }
                LRESULT(0)
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED => {
                if let Some(app) = app_of(hwnd) {
                    app.rebuild_monitors = true;
                }
                request_render(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                // 画布销毁 ≠ 引擎退出：显示器变化（WM_DISPLAYCHANGE）时
                // 会销毁旧画布重建新画布，无条件 PostQuitMessage 会把渲染线程
                // 一起杀掉（宿主实测前科）。
                // 重建路径先 unbind（GWLP_USERDATA 清 0），故 app_of 返回 None
                // 是双保险；shutdown 的写入在渲染线程，wnd_proc 经 DispatchMessageW
                // 同线程调用，bool 无跨线程可见性问题。
                if app_of(hwnd).is_some_and(|app| app.shutdown) {
                    PostQuitMessage(0);
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                // ESC -> Shutdown：正常情况下这是死路径，因为画布 WS_EX_NOACTIVATE
                // 不接收键盘消息。set_focus_hwnd 的守卫确保画布永不经库获得焦点，
                // 因此此分支仅在 standalone 调试（Win32 API 直调 SetFocus 给画布）时可达，
                // 作为逃生门保留。详见 research/esc-and-focus-guard.md。
                if wp.0 == 0x1B {
                    if let Some(app) = app_of(hwnd) {
                        app.shared.push(Command::Shutdown);
                    }
                    return LRESULT(0);
                }
                request_render(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

unsafe fn request_render(hwnd: HWND) {
    if let Some(app) = app_of(hwnd) {
        app.shared.request_render();
    }
}

// 焦点相关 FFI（SetFocus/AttachThreadInput）已于 2026-10-26 移除：跨线程代调
// SetFocus 无安全实现（attach 挂死渲染线程，实测），焦点语义改为 FocusHint 事件。

/// 统一指针事件分发：坐标换算 -> 命中测试 -> sink 查询 -> 状态机 -> 执行动作。
unsafe fn dispatch_pointer(
    hwnd: HWND,
    phase: PointerPhase,
    button: Option<PointerButton>,
    wheel_delta: i32,
) {
    let Some(app) = app_of(hwnd) else { return };
    let c = app.read_cursor();
    app.cursor = c;

    let hit_resize = app.manager.hit_resize(c[0], c[1]);
    let hit_test = if hit_resize.is_none() {
        app.manager.hit_test(c[0], c[1])
    } else {
        None
    };

    let phase_kind = match phase {
        PointerPhase::Down => PhaseKind::Down,
        PointerPhase::Up(_) => PhaseKind::Up,
        PointerPhase::Move => PhaseKind::Move,
        PointerPhase::DoubleClick => PhaseKind::DoubleClick,
        PointerPhase::RightDown => PhaseKind::RightDown,
        PointerPhase::Wheel => PhaseKind::Wheel,
    };

    let sink_decision = if phase_kind == PhaseKind::Down {
        hit_test.and_then(|id| {
            app.sink_for(id).map(|sink| {
                if let Some(b) = app.manager.get(id) {
                    let ev = PointerEvent {
                        block_local: [c[0] - b.x, c[1] - b.y],
                        phase,
                        button,
                        wheel_delta,
                    };
                    match sink.on_pointer(id, &ev) {
                        crate::api::input::InputDecision::Consumed => SinkDecision::Consumed,
                        crate::api::input::InputDecision::DragMove => SinkDecision::DragMove,
                        crate::api::input::InputDecision::Ignored => SinkDecision::Ignored,
                    }
                } else {
                    SinkDecision::Ignored
                }
            })
        })
    } else {
        None
    };

    let input = RouteInput {
        hit_resize,
        hit_test,
        sink_decision,
        phase: phase_kind,
    };
    let result = routing::route(app.route_state, input);
    let prev_state = app.route_state;
    app.route_state = result.state;

    match result.action {
        RouteAction::StartResize { block, edges } => {
            start_resize_session(app, hwnd, block, edges, c);
        }
        RouteAction::StartMove { block } => {
            start_move_session(app, hwnd, block, c);
        }
        RouteAction::Consume => {
            let was_pressing = matches!(prev_state, RouteState::Pressing(_));
            if !was_pressing {
                if let Some(hit_id) = hit_test {
                    // 抬层（z-order + window level）
                    if app.manager.bring_to_front(hit_id) {
                        app.geometry_changed = true;
                    }
                    if app.level.raises_on_click() {
                        level::raise_on_click(hwnd);
                    }
                    // FR1 焦点语义（2026-10-26 裁决）：库侧绝不代调 SetFocus/AttachThreadInput。
                    // 跨线程代焦点在 Win32 没有安全实现——attach 对无消息泵的目标线程会挂死
                    // 渲染线程（实测：插桩走到 consume_focus_enter 后心跳当场停摆，进程活着但
                    // 渲染线程冻结）。改为发 FocusHint：宿主在自己线程（poll_events）里对自己的
                    // 壳窗口 SetFocus，同线程调用合法且必成。守卫（apply_one）仍保证登记的不是
                    // 画布自身——画布永不经任何路径获得键盘。
                    if let Some(focus_hwnd) = app.focus_hwnd_for(hit_id) {
                        app.shared.emit(EngineEvent::FocusHint {
                            id: hit_id,
                            hwnd: focus_hwnd,
                        });
                    }
                }
                app.press = 1.0;
                app.shared.request_render();
            }
        }
        RouteAction::Forward { sink_block } => {
            if let Some(sink) = app.sink_for(sink_block) {
                if let Some(b) = app.manager.get(sink_block) {
                    let ev = PointerEvent {
                        block_local: [c[0] - b.x, c[1] - b.y],
                        phase,
                        button,
                        wheel_delta,
                    };
                    let _ = sink.on_pointer(sink_block, &ev);
                }
            }
        }
        RouteAction::Noop => {}
    }

    // 左键 Up 的会话收尾必须与路由结果无关：0.4.2 语义里 WM_LBUTTONUP 无条件结束
    // 拖拽/缩放并 ReleaseCapture。把收尾绑在 Forward 分支会漏掉「把手带缩放 Down
    // 不设 owner => Up 走 Noop」的路径（verify_input_sink.py 实测：缩放会话不结束，
    // 块被停放光标一路拉伸到屏幕极限）。右键 Up（Up(Some(Right))）不参与左键会话收尾。
    if phase_kind == PhaseKind::Up && button != Some(PointerButton::Right) {
        app.press = 0.0;
        if let Some(session) = app.drag.take() {
            let id = session.id();
            let resized = session.is_resize();
            if let Some(b) = app.manager.get(id) {
                let s = b.scale.max(1e-3);
                let rect = [b.x, b.y, b.width / s, b.height / s];
                app.shared.emit(if resized {
                    EngineEvent::ResizeEnded { id, rect }
                } else {
                    EngineEvent::MoveEnded { id, rect }
                });
            }
            app.hover_handle = ResizeEdges::NONE;
            app.geometry_changed = true;
            for m in app.monitors.iter_mut() {
                m.region_dirty = true;
            }
        }
        let _ = ReleaseCapture();
        request_render(hwnd);
    }
}

/// Move 事件转发给 sink。
unsafe fn forward_move_to_sink(app: &mut App) {
    let c = app.cursor;
    let target = match app.route_state {
        RouteState::Pressing(owner) => Some(owner.block),
        RouteState::Idle => app.manager.hit_test(c[0], c[1]),
    };
    if let Some(block_id) = target {
        if let Some(sink) = app.sink_for(block_id) {
            if let Some(b) = app.manager.get(block_id) {
                let ev = PointerEvent {
                    block_local: [c[0] - b.x, c[1] - b.y],
                    phase: PointerPhase::Move,
                    button: None,
                    wheel_delta: 0,
                };
                let _ = sink.on_pointer(block_id, &ev);
            }
        }
    }
}

/// 开始缩放会话。
unsafe fn start_resize_session(
    app: &mut App,
    hwnd: HWND,
    block: u32,
    edges: ResizeEdges,
    c: [f32; 2],
) {
    if app.manager.bring_to_front(block) {
        app.geometry_changed = true;
    }
    if app.level.raises_on_click() {
        level::raise_on_click(hwnd);
    }
    let Some(b) = app.manager.get(block) else {
        return;
    };
    let s = b.scale.max(1e-3);
    let origin = [b.x, b.y, b.width / s, b.height / s];
    let min_size = [b.resize.min_width, b.resize.min_height];
    let now = app.t0.elapsed().as_secs_f64();
    app.min_drag_size = min_size;
    app.drag = Some(DragSession::Resize(ResizeState::begin(
        block, edges, c, origin, now,
    )));
    app.hover = Some(block);
    app.hover_handle = edges;
    app.press = 1.0;
    let _ = SetCapture(hwnd);
    app.shared.request_render();
    if let Some(handle) = edges.public() {
        app.shared
            .emit(EngineEvent::ResizeStarted { id: block, handle });
    }
}

/// 开始移动会话。
unsafe fn start_move_session(app: &mut App, hwnd: HWND, block: u32, c: [f32; 2]) {
    if let Some(b) = app.manager.get(block) {
        if !b.movable {
            if app.manager.bring_to_front(block) {
                app.geometry_changed = true;
            }
            if app.level.raises_on_click() {
                level::raise_on_click(hwnd);
            }
            app.shared.request_render();
            return;
        }
    }
    if app.manager.bring_to_front(block) {
        app.geometry_changed = true;
    }
    if app.level.raises_on_click() {
        level::raise_on_click(hwnd);
    }
    let Some(b) = app.manager.get(block) else {
        return;
    };
    let s = b.scale.max(1e-3);
    let origin = [b.x, b.y, b.width / s, b.height / s];
    let min_size = [b.resize.min_width, b.resize.min_height];
    let now = app.t0.elapsed().as_secs_f64();
    app.min_drag_size = min_size;
    app.drag = Some(DragSession::Move(MoveState::begin(block, c, origin, now)));
    app.hover = Some(block);
    app.hover_handle = ResizeEdges::NONE;
    app.press = 1.0;
    let _ = SetCapture(hwnd);
    app.shared.request_render();
    app.shared.emit(EngineEvent::MoveStarted { id: block });
}

/// 把画布窗口区域限制成「所有块 ⊕ 外扩量」的并集。
///
/// `SetWindowRgn` 会接管成功赋值的区域对象所有权。
pub(crate) fn update_region(mon: &MonitorRt, margin: f32) {
    let width = mon.compositor.width.max(0);
    let height = mon.compositor.height.max(0);
    let margin = margin.max(0.0);
    unsafe {
        let mut region = None;
        for block in mon.blocks.iter() {
            let left = (block.x - margin).floor().max(0.0) as i32;
            let top = (block.y - margin).floor().max(0.0) as i32;
            let right = (block.x + block.width + margin).ceil().min(width as f32) as i32;
            let bottom = (block.y + block.height + margin).ceil().min(height as f32) as i32;
            if left >= right || top >= bottom {
                continue;
            }
            let rect = CreateRectRgn(left, top, right, bottom);
            if rect.0.is_null() {
                continue;
            }
            if let Some(destination) = region {
                if CombineRgn(destination, destination, rect, RGN_OR) == GDI_REGION_TYPE(0) {
                    let _ = DeleteObject(rect);
                    let _ = DeleteObject(destination);
                    region = None;
                    break;
                }
                let _ = DeleteObject(rect);
            } else {
                region = Some(rect);
            }
        }

        let Some(region) = region else {
            // 空区域必须真的空：传 NULL 会恢复整窗矩形、把桌面重新挡死，
            // 因为 WS_EX_TRANSPARENT 不是命中测试契约。
            let empty = CreateRectRgn(0, 0, 0, 0);
            if !empty.0.is_null() && SetWindowRgn(mon.hwnd, empty, true) == 0 {
                let _ = DeleteObject(empty);
            }
            return;
        };

        if SetWindowRgn(mon.hwnd, region, true) == 0 {
            let _ = DeleteObject(region);
        }
    }
}

/// 拖拽期间把窗口区域一次性扩到整窗。
///
/// 块外内容始终是全透明，扩大区域不改变任何像素；命中测试仍由 `WM_NCHITTEST` 的
/// `HTTRANSPARENT` 负责穿透。换来的是拖拽全程 0 次 `SetWindowRgn`。
pub(crate) fn set_region_full(mon: &MonitorRt) {
    unsafe {
        let width = mon.compositor.width.max(1);
        let height = mon.compositor.height.max(1);
        let full = CreateRectRgn(0, 0, width, height);
        if full.0.is_null() {
            return;
        }
        if SetWindowRgn(mon.hwnd, full, false) == 0 {
            let _ = DeleteObject(full);
        }
    }
}

/// 画布可见性 = 采集就绪 且 未被独占全屏压制 且 本屏非空。
pub(crate) fn apply_visibility(mon: &MonitorRt) {
    unsafe {
        if mon.ready && !mon.hidden_fs && !mon.hidden_capture && !mon.hidden_empty {
            mon.show();
        } else {
            let _ = ShowWindow(mon.hwnd, SW_HIDE);
        }
    }
}

/// 点击穿透切换（方案 12.1.4）：光标在本屏某块内 → 取消穿透；否则恢复。
pub(crate) fn set_click_through(hwnd: HWND, inside: bool) {
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as i64;
        let transparent = (style & WS_EX_TRANSPARENT.0 as i64) != 0;
        if inside && transparent {
            SetWindowLongPtrW(
                hwnd,
                GWL_EXSTYLE,
                (style & !(WS_EX_TRANSPARENT.0 as i64)) as isize,
            );
        } else if !inside && !transparent {
            SetWindowLongPtrW(
                hwnd,
                GWL_EXSTYLE,
                (style | WS_EX_TRANSPARENT.0 as i64) as isize,
            );
        }
    }
}

/// 该 HWND 是不是我们自己的画布（类名固定）。
///
/// 用来避免把自家 overlay 误判成独占全屏应用 —— 画布本身就是 `WS_POPUP` + 铺满显示器，
/// 完全符合「全屏」的形状判据。
///
/// ⚠ 误判的后果被 A（`hidden_fs` 停产门）放大了：把**自家画布**认成「独占全屏应用」
/// 就会整屏停采 + 停产，玻璃直接消失，而且不会报成我们的错误。所以纯比较逻辑
/// 抽成 [`class_name_is_canvas`] 单独测 —— 真实 HWND 在 CI 里造不出来。
#[must_use]
pub(crate) fn is_own_window(hwnd: HWND) -> bool {
    if hwnd.0.is_null() {
        return false;
    }
    let mut buf = [0u16; 64];
    // `GetClassNameW` 返回**写入的字符数**（不含结尾 0），失败返回 0，两者都 <= buf.len()。
    // 仍然 clamp 一次：本函数现在也被 `platform::occlusion` 的 `EnumWindows` 回调以 8 Hz
    // 对桌面上**每个**窗口调用，而回调里 panic 穿 FFI + release profile 的 `panic = "abort"`
    // 会直接带走整个渲染进程 ⇒「切片不越界」必须是结构性质，不是对 Win32 返回值的信任。
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    class_name_is_canvas(&buf, n)
}

/// 类名比较的纯逻辑：`GetClassNameW` 写进 `buf` 的前 `n` 个宽字符，是否恰好是我们的画布类名。
///
/// **零分配**：直接比宽字符常量，不再 `String::from_utf16_lossy`（那是一次堆分配，
/// 而任务硬约束要求「`EnumWindows` 回调里不分配 `String` / `Vec`」；本函数被那条回调
/// 对桌面上每个窗口调用一次，4 Hz 的 `fullscreen_app_on` 也走它）。
/// 判据来源仍然是同一个 [`CANVAS_CLASS_NAME`]（编译期展开成 `CANVAS_CLASS_NAME_W`，
/// 单一来源，见其注释）。
///
/// `n` 是 `GetClassNameW` 的原始 `i32` 返回值：负数（不该出现）按 0 处理，超过 `buf`
/// 长度按 `buf.len()` 截断 —— 两条都用 saturating/clamp 表达，不留越界切片的余地。
#[must_use]
fn class_name_is_canvas(buf: &[u16], n: i32) -> bool {
    let n = (n.max(0) as usize).min(buf.len());
    // 两者的长度约定差一个结尾 0：`CANVAS_CLASS_NAME_W` **含**它，
    // 而 `GetClassNameW` 的计数**不含**，所以只比 len-1 那一段。
    let expect = &CANVAS_CLASS_NAME_W[..CANVAS_CLASS_NAME_W.len().saturating_sub(1)];
    &buf[..n] == expect
}

#[cfg(test)]
mod tests {
    use super::{class_name_is_canvas, CANVAS_CLASS_NAME, CANVAS_CLASS_NAME_W};

    #[test]
    fn class_name_is_canvas_accepts_only_the_exact_name() {
        // 这条判据错一次的后果：`fullscreen_app_on` 把自家画布当成独占全屏应用
        // ⇒ `hidden_fs` ⇒ 整屏停采停产 ⇒ 玻璃永久消失（A 放大了误判的代价）。
        let name = &CANVAS_CLASS_NAME_W[..CANVAS_CLASS_NAME_W.len() - 1];
        let mut buf = [0u16; 64];
        buf[..name.len()].copy_from_slice(name);
        assert!(
            class_name_is_canvas(&buf, name.len() as i32),
            "精确类名必须认成自家画布"
        );
        // 少一个字符 / 多一个结尾 0 / 空串 / 别的窗口类，一律不是自家画布
        assert!(!class_name_is_canvas(&buf, name.len() as i32 - 1));
        assert!(!class_name_is_canvas(&buf, name.len() as i32 + 1));
        assert!(!class_name_is_canvas(&buf, 0));
        let mut other = [0u16; 64];
        let o: Vec<u16> = "Progman".encode_utf16().collect();
        other[..o.len()].copy_from_slice(&o);
        assert!(!class_name_is_canvas(&other, o.len() as i32));
        // Win32 不该给出这两种返回值，但「不可能 panic」必须是结构性质：
        // 负数按 0 处理，超长按 buf.len() 截断，两者都只影响结论不影响安全性。
        assert!(!class_name_is_canvas(&buf, -1));
        assert!(!class_name_is_canvas(&buf, i32::MIN));
        assert!(!class_name_is_canvas(&buf, i32::MAX));
    }

    #[test]
    fn wide_class_name_matches_narrow_and_is_null_terminated() {
        let narrow: Vec<u16> = CANVAS_CLASS_NAME.encode_utf16().collect();
        assert_eq!(CANVAS_CLASS_NAME_W.len(), narrow.len() + 1);
        assert_eq!(&CANVAS_CLASS_NAME_W[..narrow.len()], narrow.as_slice());
        assert_eq!(
            CANVAS_CLASS_NAME_W[narrow.len()],
            0,
            "必须以 0 结尾，RegisterClassExW 才认"
        );
    }

    #[test]
    fn class_name_is_not_the_legacy_one() {
        // 重构前叫 LiquidGlassCanvas；改名后 is_own_window 的判据必须同步
        assert_eq!(CANVAS_CLASS_NAME, "LiquidVerreCanvas");
        assert!(!CANVAS_CLASS_NAME.contains("Glass"));
    }

    #[test]
    fn is_class_already_exists_only_matches_0x80070582() {
        assert!(super::is_class_already_exists(0x8007_0582u32 as i32));
        assert!(!super::is_class_already_exists(0x8007_0005u32 as i32));
        assert!(!super::is_class_already_exists(1410));
        assert!(!super::is_class_already_exists(0));
    }
}
