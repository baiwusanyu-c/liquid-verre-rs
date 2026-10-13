//! 中文控制面板（内嵌 HTTP 服务器 + 系统浏览器）
//! 全部折射参数走 slider（现代化 UI）；
//! 玻璃块可增删，尺寸与圆角可改。

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Arc;
use std::thread;

use tiny_http::{Response, Server};

use crate::glass::params::Params;

/// 面板与主渲染线程共享的参数（原子存储，避免锁）
#[derive(Debug, Clone)]
pub struct SharedParams {
    pub radius: Arc<AtomicIsize>,
    pub lens_coverage: Arc<AtomicIsize>,
    pub ior: Arc<AtomicIsize>,
    pub thickness: Arc<AtomicIsize>,
    pub refr_strength: Arc<AtomicIsize>,
    pub dispersion: Arc<AtomicIsize>,
    pub clarity: Arc<AtomicIsize>,
    pub blur_max: Arc<AtomicIsize>,
    pub spec_intensity: Arc<AtomicIsize>,
    pub spec_power: Arc<AtomicIsize>,
    pub fresnel: Arc<AtomicIsize>,
    pub caustic: Arc<AtomicIsize>,
    pub tint_opacity: Arc<AtomicIsize>,
    pub adaptive_tint: Arc<AtomicIsize>,
    pub mouse_intensity: Arc<AtomicIsize>,
    pub mouse_radius: Arc<AtomicIsize>,
    pub shadow_opacity: Arc<AtomicIsize>,
    /// 待执行的块操作：1=加一块 2=删一块 3=清空 4=加20块
    pub block_action: Arc<AtomicIsize>,
    pub new_width: Arc<AtomicIsize>,
    pub new_height: Arc<AtomicIsize>,
    /// 当前选中的块（列表选中项）
    pub selected_id: Arc<AtomicIsize>,
}

impl Default for SharedParams {
    fn default() -> Self {
        let d = Params::default();
        Self {
            radius: Arc::new(AtomicIsize::new((d.corner_radius * 10.0) as isize)),
            lens_coverage: Arc::new(AtomicIsize::new((d.lens_coverage * 100.0) as isize)),
            ior: Arc::new(AtomicIsize::new((d.ior * 100.0) as isize)),
            thickness: Arc::new(AtomicIsize::new(d.thickness as isize)),
            refr_strength: Arc::new(AtomicIsize::new((d.refr_strength * 100.0) as isize)),
            dispersion: Arc::new(AtomicIsize::new((d.dispersion * 100.0) as isize)),
            clarity: Arc::new(AtomicIsize::new((d.clarity * 100.0) as isize)),
            blur_max: Arc::new(AtomicIsize::new(d.blur_max as isize)),
            spec_intensity: Arc::new(AtomicIsize::new((d.spec_intensity * 100.0) as isize)),
            spec_power: Arc::new(AtomicIsize::new(d.spec_power as isize)),
            fresnel: Arc::new(AtomicIsize::new((d.fresnel_f0 * 1000.0) as isize)),
            caustic: Arc::new(AtomicIsize::new((d.caustic_gain * 100.0) as isize)),
            tint_opacity: Arc::new(AtomicIsize::new((d.tint_opacity * 100.0) as isize)),
            adaptive_tint: Arc::new(AtomicIsize::new((d.adaptive_tint * 100.0) as isize)),
            mouse_intensity: Arc::new(AtomicIsize::new((d.mouse_intensity * 100.0) as isize)),
            mouse_radius: Arc::new(AtomicIsize::new(d.mouse_radius as isize)),
            shadow_opacity: Arc::new(AtomicIsize::new((d.shadow_opacity * 100.0) as isize)),
            block_action: Arc::new(AtomicIsize::new(0)),
            new_width: Arc::new(AtomicIsize::new(600)),
            new_height: Arc::new(AtomicIsize::new(400)),
            selected_id: Arc::new(AtomicIsize::new(-1)),
        }
    }
}

impl SharedParams {
    /// 把面板值落到 Params
    pub fn apply(&self, p: &mut Params) {
        p.corner_radius = self.radius.load(Ordering::Relaxed) as f32 / 10.0;
        p.lens_coverage = self.lens_coverage.load(Ordering::Relaxed) as f32 / 100.0;
        p.ior = self.ior.load(Ordering::Relaxed) as f32 / 100.0;
        p.thickness = self.thickness.load(Ordering::Relaxed) as f32;
        p.refr_strength = self.refr_strength.load(Ordering::Relaxed) as f32 / 100.0;
        p.dispersion = self.dispersion.load(Ordering::Relaxed) as f32 / 100.0;
        p.clarity = self.clarity.load(Ordering::Relaxed) as f32 / 100.0;
        p.blur_max = self.blur_max.load(Ordering::Relaxed) as f32;
        // 控制面板只有一个模糊滑杆时，保证边缘不会静默保持旧值。
        p.blur_min = (p.blur_max * 0.12).clamp(0.5, 12.0).min(p.blur_max);
        p.spec_intensity = self.spec_intensity.load(Ordering::Relaxed) as f32 / 100.0;
        p.spec_power = self.spec_power.load(Ordering::Relaxed) as f32;
        p.fresnel_f0 = self.fresnel.load(Ordering::Relaxed) as f32 / 1000.0;
        p.caustic_gain = self.caustic.load(Ordering::Relaxed) as f32 / 100.0;
        p.tint_opacity = self.tint_opacity.load(Ordering::Relaxed) as f32 / 100.0;
        p.adaptive_tint = self.adaptive_tint.load(Ordering::Relaxed) as f32 / 100.0;
        p.mouse_intensity = self.mouse_intensity.load(Ordering::Relaxed) as f32 / 100.0;
        p.mouse_radius = self.mouse_radius.load(Ordering::Relaxed) as f32;
        p.shadow_opacity = self.shadow_opacity.load(Ordering::Relaxed) as f32 / 100.0;
        p.sanitize();
    }
}

/// 启动 HTTP 控制面板
pub fn create(shared: &'static SharedParams) -> Result<(), String> {
    let server = Server::http("127.0.0.1:8080").map_err(|e| format!("HTTP 服务器启动失败: {e}"))?;

    thread::spawn(move || {
        for request in server.incoming_requests() {
            handle_request(request, shared);
        }
    });

    // 自动打开浏览器
    let _ = std::process::Command::new("cmd")
        .args(["/c", "start", "http://127.0.0.1:8080"])
        .spawn();

    Ok(())
}

fn handle_request(mut request: tiny_http::Request, shared: &SharedParams) {
    let url = request.url().to_string();
    let method = request.method().as_str();

    match (method, url.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => {
            let html = include_str!("index.html");
            let response = Response::from_string(html).with_header(
                tiny_http::Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap(),
            );
            let _ = request.respond(response);
        }
        ("POST", "/api/param") => {
            let mut body = String::new();
            if request.as_reader().read_to_string(&mut body).is_ok() {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                    if let (Some(name), Some(value)) = (
                        json.get("name").and_then(|v| v.as_str()),
                        json.get("value").and_then(|v| v.as_f64()),
                    ) {
                        update_param(name, value as f32, shared);
                    }
                }
            }
            let _ = request.respond(Response::from_string("{\"ok\":true}"));
        }
        ("POST", "/api/add") => {
            shared.block_action.store(1, Ordering::Release);
            let _ = request.respond(Response::from_string("{\"ok\":true}"));
        }
        ("POST", "/api/add20") => {
            shared.block_action.store(4, Ordering::Release);
            let _ = request.respond(Response::from_string("{\"ok\":true}"));
        }
        ("POST", "/api/remove") => {
            shared.block_action.store(2, Ordering::Release);
            let _ = request.respond(Response::from_string("{\"ok\":true}"));
        }
        ("POST", "/api/clear") => {
            shared.block_action.store(3, Ordering::Release);
            let _ = request.respond(Response::from_string("{\"ok\":true}"));
        }
        ("POST", "/api/size") => {
            let mut body = String::new();
            if request.as_reader().read_to_string(&mut body).is_ok() {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                    if let (Some(w), Some(h)) = (
                        json.get("width").and_then(|v| v.as_f64()),
                        json.get("height").and_then(|v| v.as_f64()),
                    ) {
                        let width = (w as isize).clamp(24, 8000);
                        let height = (h as isize).clamp(24, 8000);
                        shared.new_width.store(width, Ordering::Relaxed);
                        shared.new_height.store(height, Ordering::Relaxed);
                    }
                }
            }
            let _ = request.respond(Response::from_string("{\"ok\":true}"));
        }
        ("GET", "/api/params") => {
            let json = serde_json::json!({
                "radius": shared.radius.load(Ordering::Relaxed) as f32 / 10.0,
                "lens_coverage": shared.lens_coverage.load(Ordering::Relaxed) as f32 / 100.0,
                "ior": shared.ior.load(Ordering::Relaxed) as f32 / 100.0,
                "thickness": shared.thickness.load(Ordering::Relaxed) as f32,
                "refr_strength": shared.refr_strength.load(Ordering::Relaxed) as f32 / 100.0,
                "dispersion": shared.dispersion.load(Ordering::Relaxed) as f32 / 100.0,
                "clarity": shared.clarity.load(Ordering::Relaxed) as f32 / 100.0,
                "blur_max": shared.blur_max.load(Ordering::Relaxed) as f32,
                "spec_intensity": shared.spec_intensity.load(Ordering::Relaxed) as f32 / 100.0,
                "spec_power": shared.spec_power.load(Ordering::Relaxed) as f32,
                "fresnel": shared.fresnel.load(Ordering::Relaxed) as f32 / 1000.0,
                "caustic": shared.caustic.load(Ordering::Relaxed) as f32 / 100.0,
                "tint_opacity": shared.tint_opacity.load(Ordering::Relaxed) as f32 / 100.0,
                "adaptive_tint": shared.adaptive_tint.load(Ordering::Relaxed) as f32 / 100.0,
                "mouse_intensity": shared.mouse_intensity.load(Ordering::Relaxed) as f32 / 100.0,
                "mouse_radius": shared.mouse_radius.load(Ordering::Relaxed) as f32,
                "shadow_opacity": shared.shadow_opacity.load(Ordering::Relaxed) as f32 / 100.0,
                "new_width": shared.new_width.load(Ordering::Relaxed) as f32,
                "new_height": shared.new_height.load(Ordering::Relaxed) as f32,
            });
            let response = Response::from_string(json.to_string()).with_header(
                tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
            );
            let _ = request.respond(response);
        }
        _ => {
            let _ = request.respond(Response::from_string("Not Found").with_status_code(404));
        }
    }
}

fn update_param(name: &str, value: f32, shared: &SharedParams) {
    match name {
        "radius" => shared
            .radius
            .store((value * 10.0) as isize, Ordering::Relaxed),
        "lens_coverage" => shared
            .lens_coverage
            .store((value * 100.0) as isize, Ordering::Relaxed),
        "ior" => shared
            .ior
            .store((value * 100.0) as isize, Ordering::Relaxed),
        "thickness" => shared.thickness.store(value as isize, Ordering::Relaxed),
        "refr_strength" => shared
            .refr_strength
            .store((value * 100.0) as isize, Ordering::Relaxed),
        "dispersion" => shared
            .dispersion
            .store((value * 100.0) as isize, Ordering::Relaxed),
        "clarity" => shared
            .clarity
            .store((value * 100.0) as isize, Ordering::Relaxed),
        "blur_max" => shared.blur_max.store(value as isize, Ordering::Relaxed),
        "spec_intensity" => shared
            .spec_intensity
            .store((value * 100.0) as isize, Ordering::Relaxed),
        "spec_power" => shared.spec_power.store(value as isize, Ordering::Relaxed),
        "fresnel" => shared
            .fresnel
            .store((value * 1000.0) as isize, Ordering::Relaxed),
        "caustic" => shared
            .caustic
            .store((value * 100.0) as isize, Ordering::Relaxed),
        "tint_opacity" => shared
            .tint_opacity
            .store((value * 100.0) as isize, Ordering::Relaxed),
        "adaptive_tint" => shared
            .adaptive_tint
            .store((value * 100.0) as isize, Ordering::Relaxed),
        "mouse_intensity" => shared
            .mouse_intensity
            .store((value * 100.0) as isize, Ordering::Relaxed),
        "mouse_radius" => shared.mouse_radius.store(value as isize, Ordering::Relaxed),
        "shadow_opacity" => shared
            .shadow_opacity
            .store((value * 100.0) as isize, Ordering::Relaxed),
        _ => {}
    }
}
