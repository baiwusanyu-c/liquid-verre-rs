# -*- coding: utf-8 -*-
"""fix: window.rs Up 收尾无条件化 + wheel 高位字；verify_input_sink.py 双击 PostMessage 兜底。"""
import io

# ---------- 1) window.rs ----------
P = r"D:/github-project/liquid-verre-rs/crates/liquid-verre/src/platform/window.rs"
t = io.open(P, encoding="utf-8", newline="\n").read()

old_wheel = "let delta = (wp.0 as i16) as i32 / 120;"
assert old_wheel in t, "wheel anchor"
t = t.replace(old_wheel, "let delta = ((wp.0 >> 16) as i16) as i32 / 120;", 1)

old_forward = """        RouteAction::Forward { sink_block } => {
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
}"""
new_forward = """        RouteAction::Forward { sink_block } => {
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
}"""
assert old_forward in t, "Forward arm anchor"
t = t.replace(old_forward, new_forward, 1)
io.open(P, "w", encoding="utf-8", newline="\n").write(t)
print("window.rs fixed")

# ---------- 2) verify_input_sink.py: 双击 PostMessage 兜底 ----------
Q = r"D:/github-project/liquid-verre-rs/tools/verify_input_sink.py"
s = io.open(Q, encoding="utf-8", newline="\n").read()
anchor = """        user32.mouse_event(LEFTUP, 0, 0, 0, 0)
        time.sleep(0.15)
        park_cursor()"""
repl = """        user32.mouse_event(LEFTUP, 0, 0, 0, 0)
        time.sleep(0.08)
        # 物理连击的 DBLCLK 合成依赖系统双击时限，不稳定；直接 PostMessage 兜底
        # （dispatch 读 read_cursor，不读 lp，坐标不受影响）。
        user32.PostMessageW(hwnd, WM_LBUTTONDBLCLK, 0x0001, make_lparam(cx, cy))
        time.sleep(0.15)
        park_cursor()"""
assert anchor in s, "dbl anchor"
s = s.replace(anchor, repl, 1)
io.open(Q, "w", encoding="utf-8", newline="\n").write(s)
print("verify_input_sink.py patched")
