//! 内嵌 HTTP 控制面板（tiny_http + 系统浏览器）。
//!
//! 全部读写都走 `liquid_verre` 的**公共 API**：
//! - 读：`Engine::snapshot()` / `window_snapshots()` / `monitors()`；
//! - 写：`Engine::set_default_style` / `create_window` / `GlassWindow::set_*`。
//!
//! 面板不持有任何 GPU 资源，也不需要原子量中转 —— 重构前那套
//! `SharedParams { radius: Arc<AtomicIsize>, … }` 已经整体删除，
//! 因为它只暴露了 40 个参数里的 17 个，而且是库内部类型（违反分层）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use liquid_verre::{
    Engine, GlassMode, GlassStyle, LiquidParams, ResizeOptions, TransparentParams, WindowConfig,
    WindowLevel, ZOrder,
};
use serde_json::{json, Value};
use tiny_http::{Header, Request, Response, Server};

use crate::schema;

/// 面板句柄。`stop()` 只是停止接受新连接；已建立的连接会自然结束。
pub struct Panel {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Panel {
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// 启动面板。`port = 0` 表示让操作系统挑一个空闲端口。
pub fn start(engine: Arc<Engine>, port: u16, no_browser: bool) -> Result<Panel, String> {
    let server = Server::http(format!("127.0.0.1:{port}")).map_err(|e| e.to_string())?;
    let actual = server
        .server_addr()
        .to_ip()
        .map(|a| a.port())
        .unwrap_or(port);
    let url = format!("http://127.0.0.1:{actual}");
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);
    let url2 = url.clone();

    let handle = std::thread::Builder::new()
        .name("playground-panel".into())
        .spawn(move || {
            // 非阻塞轮询：tiny_http 没有带超时的 recv，用停标志 + 短超时轮询。
            for request in server.incoming_requests() {
                if stop2.load(Ordering::Acquire) {
                    break;
                }
                if let Err(e) = handle_request(request, &engine) {
                    eprintln!("[panel] 请求处理失败: {e}");
                }
            }
        })
        .map_err(|e| e.to_string())?;

    println!("[panel] 控制面板: {url2}");
    if !no_browser {
        open_browser(&url2);
    }
    let _ = url;
    Ok(Panel {
        stop,
        handle: Some(handle),
    })
}

fn open_browser(url: &str) {
    // Windows: cmd /c start；macOS: open；Linux: xdg-open
    let (prog, args): (&str, Vec<String>) = if cfg!(target_os = "windows") {
        (
            "cmd",
            vec!["/c".into(), "start".into(), "".into(), url.to_string()],
        )
    } else if cfg!(target_os = "macos") {
        ("open", vec![url.to_string()])
    } else {
        ("xdg-open", vec![url.to_string()])
    };
    use std::os::windows::process::CommandExt;
    let mut cmd = std::process::Command::new(prog);
    cmd.args(&args);
    if cfg!(target_os = "windows") {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    if let Err(e) = cmd.spawn() {
        eprintln!("[panel] 打开浏览器失败（可手动访问 {url}）: {e}");
    }
}

fn json_header() -> Header {
    Header::from_bytes("Content-Type", "application/json; charset=utf-8").unwrap()
}

fn respond_json(request: Request, value: Value) {
    let body = value.to_string();
    let _ = request.respond(Response::from_string(body).with_header(json_header()));
}

fn read_body(request: &mut Request) -> Value {
    let mut body = String::new();
    if request.as_reader().read_to_string(&mut body).is_err() {
        return Value::Null;
    }
    serde_json::from_str(&body).unwrap_or(Value::Null)
}

fn handle_request(mut request: Request, engine: &Arc<Engine>) -> Result<(), String> {
    let path = request.url().split('?').next().unwrap_or("/").to_string();
    match (request.method().as_str(), path.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => {
            let html = include_str!("web/index.html");
            let header = Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap();
            let _ = request.respond(Response::from_string(html).with_header(header));
        }
        ("GET", "/api/schema") => {
            respond_json(
                request,
                json!({
                    "liquid": schema::liquid_fields(),
                    "transparent": schema::transparent_fields(),
                    "resize": schema::resize_fields(),
                    "levels": ["alwaysOnTop", "normal", "clickToFront", "alwaysOnBottom"],
                }),
            );
        }
        ("GET", "/api/state") => respond_json(request, state(engine)),
        ("POST", "/api/style") => {
            let body = read_body(&mut request);
            apply_style(engine, &body);
            respond_json(request, json!({ "ok": true }));
        }
        ("POST", "/api/mode") => {
            let body = read_body(&mut request);
            apply_mode(engine, &body);
            respond_json(request, json!({ "ok": true }));
        }
        ("POST", "/api/resize") => {
            let body = read_body(&mut request);
            apply_resize(engine, &body);
            respond_json(request, json!({ "ok": true }));
        }
        ("POST", "/api/window") => {
            let body = read_body(&mut request);
            let result = apply_window(engine, &body);
            respond_json(request, result);
        }
        ("POST", "/api/level") => {
            let body = read_body(&mut request);
            if let Some(l) = body
                .get("level")
                .and_then(|v| v.as_str())
                .and_then(parse_level)
            {
                engine.set_window_level(l);
            }
            respond_json(
                request,
                json!({ "ok": true, "level": engine.window_level() }),
            );
        }
        ("GET", "/api/events") => {
            let events: Vec<Value> = engine
                .poll_events()
                .iter()
                .map(|e| json!({ "event": format!("{e:?}") }))
                .collect();
            respond_json(request, json!({ "events": events }));
        }
        _ => {
            let _ = request.respond(Response::from_string("Not Found").with_status_code(404));
        }
    }
    Ok(())
}

fn parse_level(s: &str) -> Option<WindowLevel> {
    match s {
        "alwaysOnTop" | "top" => Some(WindowLevel::AlwaysOnTop),
        "normal" => Some(WindowLevel::Normal),
        "clickToFront" | "click" => Some(WindowLevel::ClickToFront),
        "alwaysOnBottom" | "bottom" => Some(WindowLevel::AlwaysOnBottom),
        _ => None,
    }
}

/// `scope` = "default"（改引擎默认样式，所有继承它的窗口一起变）
/// 或 "window"（只改 `id` 指定的那一块，改完就不再继承默认）。
fn target_window(engine: &Engine, body: &Value) -> Option<liquid_verre::GlassWindow> {
    let id = body.get("id").and_then(|v| v.as_u64())? as u32;
    let w = engine.window(id);
    w.is_alive().then_some(w)
}

fn apply_style(engine: &Engine, body: &Value) {
    let Some(params) = body.get("params") else {
        return;
    };
    let scope = body
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    if scope == "window" {
        if let Some(w) = target_window(engine, body) {
            if let Ok(p) = serde_json::from_value::<LiquidParams>(params.clone()) {
                w.set_style(Some(GlassStyle::Liquid(p)));
            }
        }
        return;
    }
    let current = engine.default_style();
    let base = match current {
        GlassStyle::Liquid(p) => p,
        GlassStyle::Transparent(_) => LiquidParams::default(),
    };
    // 部分更新：把面板传来的字段覆盖到当前值上（serde 的 flatten-merge 语义）
    let mut merged = serde_json::to_value(base).unwrap_or_default();
    if let (Some(obj), Some(patch)) = (merged.as_object_mut(), params.as_object()) {
        for (k, v) in patch {
            obj.insert(k.clone(), v.clone());
        }
    }
    if let Ok(p) = serde_json::from_value::<LiquidParams>(merged) {
        engine.set_default_style(GlassStyle::Liquid(p));
    }
}

fn apply_mode(engine: &Engine, body: &Value) {
    let mode = body
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("liquid");
    let scope = body
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let style = if mode == "transparent" {
        let mut t = match current_style(engine, body) {
            GlassStyle::Transparent(t) => t,
            GlassStyle::Liquid(p) => TransparentParams {
                corner_radius: p.corner_radius,
                opacity: p.base_alpha.clamp(0.0, 1.0),
                color: p.tint_color,
            },
        };
        if let Some(v) = body.get("cornerRadius").and_then(|v| v.as_f64()) {
            t.corner_radius = v as f32;
        }
        if let Some(v) = body.get("opacity").and_then(|v| v.as_f64()) {
            t.opacity = v as f32;
        }
        if let Some(v) = body.get("color") {
            if let Ok(c) = serde_json::from_value::<[f32; 4]>(v.clone()) {
                t.color = c;
            }
        }
        t.sanitize();
        GlassStyle::Transparent(t)
    } else {
        match current_style(engine, body) {
            GlassStyle::Liquid(p) => GlassStyle::Liquid(p),
            // 透明 -> 液态：以液态默认预设打底，只把三个共有参数搬过去，
            // 这样切回液态时立刻是一个「像样」的玻璃，而不是一块什么都不做的空壳。
            GlassStyle::Transparent(t) => GlassStyle::Liquid(LiquidParams {
                corner_radius: t.corner_radius,
                base_alpha: t.opacity,
                tint_color: t.color,
                ..LiquidParams::default()
            }),
        }
    };
    if scope == "window" {
        if let Some(w) = target_window(engine, body) {
            w.set_style(Some(style));
        }
    } else {
        engine.set_default_style(style);
    }
}

fn current_style(engine: &Engine, body: &Value) -> GlassStyle {
    if body.get("scope").and_then(|v| v.as_str()) == Some("window") {
        if let Some(w) = target_window(engine, body) {
            if let Some(s) = w.style() {
                return s;
            }
        }
    }
    engine.default_style()
}

fn apply_resize(engine: &Engine, body: &Value) {
    let Some(patch) = body.get("options") else {
        return;
    };
    let base = match body.get("scope").and_then(|v| v.as_str()) {
        Some("window") => target_window(engine, body)
            .and_then(|w| w.snapshot())
            .map(|s| s.resize)
            .unwrap_or_default(),
        _ => engine.snapshot().default_resize,
    };
    let mut merged = serde_json::to_value(base).unwrap_or_default();
    if let (Some(obj), Some(p)) = (merged.as_object_mut(), patch.as_object()) {
        for (k, v) in p {
            // feedback.* 是嵌套对象，逐字段合并
            if let (Some(sub), Some(existing)) = (
                obj.get_mut(k).and_then(|x| x.as_object_mut()),
                v.as_object(),
            ) {
                for (k2, v2) in existing.clone() {
                    sub.insert(k2, v2);
                }
            } else {
                obj.insert(k.clone(), v.clone());
            }
        }
    }
    let Ok(options) = serde_json::from_value::<ResizeOptions>(merged) else {
        return;
    };
    if let Some(w) = target_window(engine, body) {
        if body.get("scope").and_then(|v| v.as_str()) == Some("window") {
            w.set_resize_options(options);
            return;
        }
    }
    engine.set_default_resize(options);
    // 默认配置只影响新建窗口；面板上勾了「应用到全部」时逐块下发
    if body.get("applyToAll").and_then(|v| v.as_bool()) == Some(true) {
        for w in engine.windows() {
            w.set_resize_options(options);
        }
    }
}

fn apply_window(engine: &Engine, body: &Value) -> Value {
    let action = body.get("action").and_then(|v| v.as_str()).unwrap_or("");
    match action {
        "create" => {
            let fallback = [64.0f32, 64.0, 600.0, 400.0];
            let rect = match body.get("rect").and_then(|v| v.as_array()) {
                Some(r) if r.len() == 4 => [
                    r[0].as_f64().unwrap_or(f64::from(fallback[0])) as f32,
                    r[1].as_f64().unwrap_or(f64::from(fallback[1])) as f32,
                    r[2].as_f64().unwrap_or(f64::from(fallback[2])) as f32,
                    r[3].as_f64().unwrap_or(f64::from(fallback[3])) as f32,
                ],
                _ => fallback,
            };
            let style = match body.get("mode").and_then(|v| v.as_str()) {
                Some("transparent") => GlassStyle::transparent(24.0, 0.4, [1.0, 1.0, 1.0, 1.0]),
                _ => engine.default_style(),
            };
            let cfg = WindowConfig {
                rect,
                style: Some(style),
                ..Default::default()
            };
            match engine.create_window(cfg) {
                Ok(w) => json!({ "ok": true, "id": w.id() }),
                Err(e) => json!({ "ok": false, "error": e.to_string() }),
            }
        }
        "close" => {
            if let Some(w) = target_window(engine, body) {
                w.close();
                json!({ "ok": true })
            } else {
                json!({ "ok": false, "error": "窗口不存在" })
            }
        }
        "clear" => {
            engine.clear_windows();
            json!({ "ok": true })
        }
        "movable" => {
            if let Some(w) = target_window(engine, body) {
                let on = body.get("value").and_then(|v| v.as_bool()).unwrap_or(true);
                w.set_movable(on);
                json!({ "ok": true })
            } else {
                json!({ "ok": false, "error": "窗口不存在" })
            }
        }
        "front" | "back" => {
            if let Some(w) = target_window(engine, body) {
                w.set_z_order(if action == "front" {
                    ZOrder::Front
                } else {
                    ZOrder::Back
                });
                json!({ "ok": true })
            } else {
                json!({ "ok": false, "error": "窗口不存在" })
            }
        }
        "rect" => {
            if let Some(w) = target_window(engine, body) {
                if let Some(r) = body.get("rect").and_then(|v| v.as_array()) {
                    if r.len() == 4 {
                        w.set_rect([
                            r[0].as_f64().unwrap_or(0.0) as f32,
                            r[1].as_f64().unwrap_or(0.0) as f32,
                            r[2].as_f64().unwrap_or(300.0) as f32,
                            r[3].as_f64().unwrap_or(200.0) as f32,
                        ]);
                    }
                }
                json!({ "ok": true })
            } else {
                json!({ "ok": false, "error": "窗口不存在" })
            }
        }
        other => json!({ "ok": false, "error": format!("未知 action: {other}") }),
    }
}

/// 面板一次性拿到渲染所需的全部状态（一次请求，避免前端发几十个）。
fn state(engine: &Engine) -> Value {
    let snap = engine.snapshot();
    let windows: Vec<Value> = snap
        .windows
        .iter()
        .map(|w| {
            json!({
                "id": w.id,
                "rect": w.rect,
                "mode": if w.mode == GlassMode::Transparent { "transparent" } else { "liquid" },
                "style": w.style,
                "resize": w.resize,
                "zOrder": w.z_order,
                "inheritsStyle": w.inherits_style,
                "monitorIndex": w.monitor_index,
                "scale": w.scale,
                "visible": w.visible,
                "interactive": w.interactive,
                "movable": w.movable,
                "hovered": w.hovered,
                "moving": w.moving,
                "resizing": w.resizing,
            })
        })
        .collect();
    let monitors: Vec<Value> = snap
        .monitors
        .iter()
        .map(|m| {
            json!({
                "name": m.name, "origin": m.origin, "size": m.size, "dpi": m.dpi,
                "scale": m.scale, "primary": m.primary,
                "captureBackend": m.capture_backend, "canvasVisible": m.canvas_visible,
            })
        })
        .collect();
    json!({
        "defaultStyle": snap.default_style,
        "defaultResize": snap.default_resize,
        "windowLevel": snap.window_level,
        "qualityLevel": snap.quality_level,
        "fps": snap.fps,
        "refreshHz": snap.refresh_hz,
        "onBattery": snap.on_battery,
        "snapshotEpoch": snap.epoch,
        "windows": windows,
        "monitors": monitors,
        "build": liquid_verre::BUILD_INFO.summary(),
    })
}
