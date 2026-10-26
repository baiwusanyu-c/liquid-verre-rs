import io, os

path = os.path.join(os.getcwd(), "crates", "liquid-verre", "src", "platform", "window.rs")

with io.open(path, "r", encoding="utf-8", newline="") as f:
    content = f.read()

# 1. Add imports
old_imports = """use crate::api::config::WindowLevel;
use crate::api::event::EngineEvent;
use crate::api::handle::Command;
use crate::core::handle::ResizeEdges;
use crate::error::{Error, ErrorKind, Result};
use crate::input::{DragSession, MoveState, ResizeState};
use crate::platform::{display::MonitorInfo, level};
use crate::runtime::{app::App, monitor::MonitorRt};"""

new_imports = """use crate::api::config::WindowLevel;
use crate::api::event::EngineEvent;
use crate::api::handle::Command;
use crate::api::input::{PointerButton, PointerEvent, PointerPhase, SinkRef};
use crate::core::handle::ResizeEdges;
use crate::core::routing::{self, PhaseKind, RouteAction, RouteInput, RouteState, SinkDecision};
use crate::error::{Error, ErrorKind, Result};
use crate::input::{DragSession, MoveState, ResizeState};
use crate::platform::{display::MonitorInfo, level};
use crate::runtime::{app::App, monitor::MonitorRt};"""

content = content.replace(old_imports, new_imports)

# 2. Replace the wnd_proc function
old_wnd_proc = """extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_LBUTTONDOWN => {
                on_button_down(hwnd);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                // 只记光标位置；hover / 把手判定在主循环里每帧做一次
                // （方案 12.2.3 输入合并：1000 Hz 消息 → 一帧一次）。
                if let Some(app) = app_of(hwnd) {
                    app.cursor = app.read_cursor();
                }
                LRESULT(0)
            }
            WM_NCHITTEST => {
                let Some(app) = app_of(hwnd) else {
                    return LRESULT(HTTRANSPARENT as isize);
                };
                let c = app.read_cursor();
                let hit = app.drag.is_some()
                    || app.manager.hit_test(c[0], c[1]).is_some()
                    || app.manager.hit_resize(c[0], c[1]).is_some();
                LRESULT(if hit {
                    HTCLIENT as isize
                } else {
                    HTTRANSPARENT as isize
                })
            }
            WM_SETCURSOR => {
                // 只在客户区（= 命中玻璃块）改光标；其余情况交回默认处理，
                // 否则会覆盖宿主应用的光标。
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
                    }
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_LBUTTONUP => {
                on_button_up(hwnd);
                LRESULT(0)
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED => {
                // 分辨率 / 缩放 / 拔插显示器：重建每屏画布与采集。
                if let Some(app) = app_of(hwnd) {
                    app.rebuild_monitors = true;
                }
                request_render(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            WM_KEYDOWN => {
                // 画布带 WS_EX_NOACTIVATE，正常情况下拿不到键盘焦点；
                // 真的拿到时 ESC 请求引擎退出（与重构前的行为一致）。
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
}"""

new_wnd_proc = """extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
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
                let delta = (wp.0 as i16) as i32 / 120; // 一格 = +/-1
                dispatch_pointer(hwnd, PointerPhase::Wheel, None, delta);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                // 光标更新 + sink 转发（Move 事件）。
                if let Some(app) = app_of(hwnd) {
                    app.cursor = app.read_cursor();
                    // 按压粘滞 / 空闲态转发 Move 给 sink
                    forward_move_to_sink(app);
                }
                LRESULT(0)
            }
            WM_NCHITTEST => {
                let Some(app) = app_of(hwnd) else {
                    return LRESULT(HTTRANSPARENT as isize);
                };
                let c = app.read_cursor();
                // drag 会话或按压归属（Consumed）都视为命中
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
                // 优先级：把手箭头 > cursor_override > 默认箭头。
                let hit_test_code = (lp.0 as u32) & 0xffff;
                if hit_test_code == HTCLIENT {
                    if let Some(app) = app_of(hwnd) {
                        // 把手箭头优先
                        if let Some(id) = app.hover_handle.cursor_id() {
                            let cursor = LoadCursorW(None, PCWSTR(id as usize as *const u16))
                                .unwrap_or_default();
                            if !cursor.0.is_null() {
                                SetCursor(cursor);
                                return LRESULT(1);
                            }
                        }
                        // cursor_override
                        let c = app.read_cursor();
                        if let Some(hit_id) = app.manager.hit_test(c[0], c[1]) {
                            if let Some(sink) = app.sink_for(hit_id) {
                                if let Some(cursor_id) = sink.cursor_override(hit_id) {
                                    let cursor = LoadCursorW(
                                        None,
                                        PCWSTR(cursor_id as usize as *const u16),
                                    )
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
                dispatch_pointer(
                    hwnd,
                    PointerPhase::Up(None),
                    Some(PointerButton::Left),
                    0,
                );
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
                PostQuitMessage(0);
                LRESULT(0)
            }
            WM_KEYDOWN => {
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
}"""

content = content.replace(old_wnd_proc, new_wnd_proc)

# 3. Add dispatch_pointer + forward_move_to_sink functions after request_render
old_request_render = """unsafe fn request_render(hwnd: HWND) {
    if let Some(app) = app_of(hwnd) {
        app.shared.request_render();
    }
}"""

new_request_render = """unsafe fn request_render(hwnd: HWND) {
    if let Some(app) = app_of(hwnd) {
        app.shared.request_render();
    }
}

/// 统一指针事件分发：坐标换算 -> 命中测试 -> sink 查询 -> 状态机 -> 执行动作。
///
/// 坐标换算复用 MOUSEMOVE 路径的 `read_cursor()` 纪律。
unsafe fn dispatch_pointer(
    hwnd: HWND,
    phase: PointerPhase,
    button: Option<PointerButton>,
    wheel_delta: i32,
) {
    let Some(app) = app_of(hwnd) else { return };
    let c = app.read_cursor();
    app.cursor = c;

    // 命中测试
    let hit_resize = app.manager.hit_resize(c[0], c[1]);
    let hit_test = if hit_resize.is_none() {
        // 把手未命中才查 hit_test（节省一次扫描）
        app.manager.hit_test(c[0], c[1])
    } else {
        None
    };

    // 转换为 routing 的 PhaseKind
    let phase_kind = match phase {
        PointerPhase::Down => PhaseKind::Down,
        PointerPhase::Up(_) => PhaseKind::Up,
        PointerPhase::Move => PhaseKind::Move,
        PointerPhase::DoubleClick => PhaseKind::DoubleClick,
        PointerPhase::RightDown => PhaseKind::RightDown,
        PointerPhase::Wheel => PhaseKind::Wheel,
    };

    // sink 决策：仅 Down 阶段且 hit_test 命中且有 sink 时查询
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

    // 状态机
    let input = RouteInput {
        hit_resize,
        hit_test,
        sink_decision,
        phase: phase_kind,
    };
    let result = routing::route(app.route_state, input);
    let prev_state = app.route_state;
    app.route_state = result.state;

    // 执行动作
    match result.action {
        RouteAction::StartResize { block, edges } => {
            start_resize_session(app, hwnd, block, edges, c);
        }
        RouteAction::StartMove { block } => {
            start_move_session(app, hwnd, block, c);
        }
        RouteAction::Consume => {
            // 记 press owner（已在 route() 中设置 route_state），不 SetCapture
            let was_pressing = matches!(prev_state, RouteState::Pressing(_));
            if !was_pressing {
                // 新按压：抬层
                if let Some(hit_id) = hit_test {
                    if app.manager.bring_to_front(hit_id) {
                        app.geometry_changed = true;
                    }
                    if app.level.raises_on_click() {
                        level::raise_on_click(hwnd);
                    }
                }
                app.press = 1.0;
                app.shared.request_render();
            }
        }
        RouteAction::Forward { sink_block } => {
            // 转发给 sink
            if let Some(sink) = app.sink_for(sink_block) {
                if let Some(b) = app.manager.get(sink_block) {
                    let ev = PointerEvent {
                        block_local: [c[0] - b.x, c[1] - b.y],
                        phase,
                        button,
                        wheel_delta,
                    };
                    // sink 返回值仅作记录，不影响库行为（非 Down 阶段）
                    let _ = sink.on_pointer(sink_block, &ev);
                }
            }
        }
        RouteAction::Noop => {}
    }
}

/// Move 事件转发给 sink：按压粘滞或空闲态。
///
/// 热路径：一次虚调用 + 平凡 struct 填充，零分配、禁格式化日志。
unsafe fn forward_move_to_sink(app: &mut App) {
    let c = app.cursor;

    // 确定目标块
    let target = match app.route_state {
        RouteState::Pressing(owner) => {
            // 按压粘滞：Move 转给 owner 块
            Some(owner.block)
        }
        RouteState::Idle => {
            // 空闲态：转发光标所在最上层块
            app.manager.hit_test(c[0], c[1])
        }
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

/// 开始缩放会话（把手命中）。
unsafe fn start_resize_session(
    app: &mut App,
    hwnd: HWND,
    block: u32,
    edges: ResizeEdges,
    c: [f32; 2],
) {
    // 抬层
    if app.manager.bring_to_front(block) {
        app.geometry_changed = true;
    }
    if app.level.raises_on_click() {
        level::raise_on_click(hwnd);
    }

    let Some(b) = app.manager.get(block) else { return };
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

/// 开始移动会话（块内命中，sink 返回 DragMove/Ignored 或无 sink）。
unsafe fn start_move_session(app: &mut App, hwnd: HWND, block: u32, c: [f32; 2]) {
    // 不可移动的块：放弃
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

    // 抬层
    if app.manager.bring_to_front(block) {
        app.geometry_changed = true;
    }
    if app.level.raises_on_click() {
        level::raise_on_click(hwnd);
    }

    let Some(b) = app.manager.get(block) else { return };
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
}"""

content = content.replace(old_request_render, new_request_render)

# 4. Remove old on_button_down and on_button_up functions (they are now inlined into dispatch/start_*)
# Find and remove them
old_down_start = """
/// 按下：先判缩放把手（角优先于边），再判整块移动。
unsafe fn on_button_down(hwnd: HWND) {"""

# Find the end of on_button_up
old_up_end = """    let _ = ReleaseCapture();
    request_render(hwnd);
}"""

# Replace on_button_down through on_button_up with nothing
# Let me find the exact text between these functions

# Actually, let me just replace the whole section. Keep the function text between
# the end of dispatch_pointer/new functions and update_region

# The old functions start with "/// 按下" and end with the closing brace of on_button_up

old_down_text_start = content.find("/// 按下：先判缩放把手")
if old_down_text_start >= 0:
    old_up_text_end = content.find(old_up_end, old_down_text_start)
    if old_up_text_end >= 0:
        old_up_text_end += len(old_up_end)
        # Remove old on_button_down and on_button_up
        content = content[:old_down_text_start] + content[old_up_text_end:]

with io.open(path, "w", encoding="utf-8", newline="") as f:
    f.write(content)

print("window.rs updated OK")