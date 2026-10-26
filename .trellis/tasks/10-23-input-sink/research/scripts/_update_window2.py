import io, os

path = os.path.join(os.getcwd(), "crates", "liquid-verre", "src", "platform", "window.rs")

with io.open(path, "r", encoding="utf-8", newline="") as f:
    lines = f.readlines()

# Replace lines 172-347 (0-indexed: 171-346)

new_code = '''extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
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
                let delta = (wp.0 as i16) as i32 / 120;
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
}

unsafe fn request_render(hwnd: HWND) {
    if let Some(app) = app_of(hwnd) {
        app.shared.request_render();
    }
}

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
            // Up 阶段结束拖拽会话
            if phase_kind == PhaseKind::Up {
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
        RouteAction::Noop => {}
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
}

'''

# Replace lines 171-346 (0-indexed) with new code
new_lines = lines[:171] + [new_code] + lines[347:]

with io.open(path, "w", encoding="utf-8", newline="") as f:
    f.writelines(new_lines)

print("window.rs updated OK")