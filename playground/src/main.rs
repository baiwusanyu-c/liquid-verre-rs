//! liquid-verre-rs 的 dev playground。
//!
//! **这不是库的一部分**：它只用 `liquid_verre` 的公共 API（`Engine` / `GlassWindow` /
//! `EngineConfig` / `EngineEvent`），因此同时兼作 API 的活文档 —— 上层应用能做的事，
//! 这里全部用公共 API 做一遍，没有一个 `pub(crate)` 后门。
//!
//! 运行：
//! ```text
//! cargo run --release --bin liquid-verre-playground
//! cargo run --release --bin liquid-verre-playground -- --port 8787 --transparent
//! ```
//! 面板默认监听 <http://127.0.0.1:8787> 并自动打开系统浏览器。

mod panel;
mod schema;

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use liquid_verre::{
    DiagnosticsConfig, Engine, EngineConfig, EngineEvent, GlassInputSink, GlassStyle,
    InputDecision, PointerEvent, PointerPhase, ResizeOptions, WindowConfig, WindowLevel,
};

/// 输入路由 demo sink：记录事件日志 + 块左半返回 Consumed、右半返回 DragMove。
/// 仅在 GLASS_INPUT_SINK_DEMO=1 时挂载（供 tools/verify_input_sink.py 取证），默认不存在。
struct SinkDemo {
    /// 判定边界（物理像素）：块宽一半以左 = Consumed。挂载时用逻辑 rect × scale 换算。
    half_w_phys: f32,
}

impl SinkDemo {
    fn new(half_w_phys: f32) -> Self {
        Self { half_w_phys }
    }
}

// GetCapture 只在拥有捕获的线程里有意义（跨线程返回 NULL）。sink 回调息在渲染线程（canvas
// wnd_proc 上下文）执行——这正是“按压后库有没有 SetCapture”的正确取证位置。
// playground 不是库，这里直接绑 user32 而不引入 windows crate（取证专用的唯一 unsafe 点）。
extern "system" {
    fn GetCapture() -> isize;
    fn GetConsoleWindow() -> isize;
    fn SetFocus(hwnd: isize) -> isize;
    fn GetCurrentThreadId() -> u32;
}

impl GlassInputSink for SinkDemo {
    fn on_pointer(&self, id: u32, event: &PointerEvent) -> InputDecision {
        let cap = unsafe { GetCapture() };
        // 左半区域返回 Consumed，右半返回 DragMove（坐标是块内物理像素）
        let decision = if event.block_local[0] < self.half_w_phys {
            InputDecision::Consumed
        } else {
            InputDecision::DragMove
        };
        // [verify] 前缀日志：供 verify_input_sink.py 取证。Down 才记决策（其余阶段返回值
        // 无语义，刷决策字段会暏日志面积）；cap= 记录事件瞬间的 capture 归属。
        let dec_tag = match event.phase {
            PointerPhase::Down => format!(" dec={:?}", decision),
            _ => String::new(),
        };
        let tid = unsafe { GetCurrentThreadId() };
        log::info!(
            target: "verify",
            "sink_demo id={} phase={:?} btn={:?} local=[{:.0},{:.0}] wheel={} cap={} tid={}{}",
            id,
            event.phase,
            event.button,
            event.block_local[0],
            event.block_local[1],
            event.wheel_delta,
            cap,
            tid,
            dec_tag,
        );
        decision
    }
}

fn main() -> ExitCode {
    // 库自己会装默认 logger（上层已装则不覆盖）。playground 不装，直接用库的。
    let opts = parse_args();

    // GLASS_* 诊断开关由库自己解析（diag::env）；playground 只负责「默认建一块玻璃」
    // 这件事不要和库的诊断布局打架：只要脚本设置了任何一种布局开关，就不再多建。
    let config = EngineConfig {
        diagnostics: DiagnosticsConfig {
            default_window: !opts.skip_default_window && !layout_from_env(),
            ..Default::default()
        },
        window_level: opts.level,
        default_style: if opts.transparent {
            GlassStyle::transparent(28.0, 0.38, [1.0, 1.0, 1.0, 1.0])
        } else {
            GlassStyle::default()
        },
        default_resize: ResizeOptions::default(),
        ..Default::default()
    };

    let engine = match Engine::spawn(config) {
        Ok(e) => Arc::new(e),
        Err(e) => {
            eprintln!("[playground] 引擎启动失败: {e}");
            return ExitCode::FAILURE;
        }
    };

    // 交互演示：额外建一块液态 + 一块透明做对照，让「同一引擎里两种模式并存」
    // 在面板上直接可见（需求 F3.4）。
    //
    // **取证 / 性能脚本运行时必须完全跳过**：tools/*.py 靠 GLASS_DEFAULT_BLOCK /
    // GLASS_MANY / GLASS_FAR_GRID / GLASS_SPREAD 得到确定性布局，多出来的窗口会让
    // 逐位比对图整体错位。这条不是猜的 —— cmp_visual.py 实测抓到过：
    // 未加门禁时 single 场景 interior 差了 49885 像素、平均通道差 50.82。
    if opts.demo_pair && !layout_from_env() && !diagnostics_from_env() {
        // 引擎刚 spawn，渲染线程可能还没发布显示器快照；等一下再取主屏尺寸，
        // 否则窗口会按 1920x1080 的兜底值摆放，在 4K 屏上偏到左上角。
        wait_for_monitors(&engine);
        let [w, h] = primary_size(&engine);
        let liquid = engine
            .create_window(WindowConfig {
                rect: [
                    w * 0.08,
                    h * 0.18,
                    (w * 0.36).clamp(320.0, 900.0),
                    (h * 0.5).clamp(240.0, 640.0),
                ],
                style: Some(GlassStyle::default()),
                ..Default::default()
            })
            .expect("create liquid window");
        let transparent = engine
            .create_window(WindowConfig {
                rect: [
                    w * 0.52,
                    h * 0.28,
                    (w * 0.36).clamp(320.0, 900.0),
                    (h * 0.42).clamp(200.0, 560.0),
                ],
                style: Some(GlassStyle::transparent(24.0, 0.35, [0.85, 0.92, 1.0, 1.0])),
                ..Default::default()
            })
            .expect("create transparent window");
        log::info!(
            "[playground] 已创建对照窗口：liquid id={} / transparent id={}",
            liquid.id(),
            transparent.id()
        );
    }

    // GlassInputSink demo 挂载：GLASS_INPUT_SINK_DEMO=1 时给「唯一默认块」接 sink 并起 rect 心跳。
    // 取证脚本用 GLASS_DEFAULT_BLOCK=1（layout_from_env 命中 ⇒ playground 自己不再建，全程恰好一块）；
    // 出现多窗口就拒绝挂载——挂错块的取证是假证据。
    if std::env::var_os("GLASS_INPUT_SINK_DEMO").is_some() {
        wait_for_monitors(&engine);
        let wins = engine.windows();
        if wins.len() != 1 {
            log::warn!(
                "[playground] GLASS_INPUT_SINK_DEMO 需要单块布局，当前 wins={}，不挂载",
                wins.len()
            );
        } else if let Some(s) = wins[0].snapshot() {
            let half = s.rect[2] * s.scale * 0.5;
            wins[0].set_input_sink(Some(Arc::new(SinkDemo::new(half))));

            // FR1 焦点 demo：GLASS_FOCUS_DEMO=1 时，Consumed 区按压后 SetFocus 到宿主代表 hwnd。
            // 优先用 GetConsoleWindow()；若不可靠退化到最小壳窗口（见 verify_focus_drop.py 焦点取证段）。
            let focus_hwnd: isize;
            if std::env::var_os("GLASS_FOCUS_DEMO").is_some() {
                let console_hwnd = unsafe { GetConsoleWindow() };
                if console_hwnd != 0 {
                    focus_hwnd = console_hwnd;
                    log::info!(
                        target: "verify",
                        "focus_demo hwnd=console(0x{:x})",
                        focus_hwnd
                    );
                } else {
                    // 退化：创建最小壳窗口作为焦点代表（FFI，不引 windows crate）
                    // GetConsoleWindow 在 subprocess 场景（stdout=PIPE）不可靠。
                    extern "system" {
                        fn CreateWindowExW(
                            dwExStyle: u32,
                            lpClassName: *const u16,
                            lpWindowName: *const u16,
                            dwStyle: u32,
                            x: i32,
                            y: i32,
                            nWidth: i32,
                            nHeight: i32,
                            hWndParent: isize,
                            hMenu: isize,
                            hInstance: isize,
                            lpParam: isize,
                        ) -> isize;
                    }
                    let class_name: Vec<u16> = "STATIC\\0".encode_utf16().collect();
                    let title: Vec<u16> = "liquid-verre-focus-host\\0".encode_utf16().collect();
                    let hwnd = unsafe {
                        CreateWindowExW(
                            0,
                            class_name.as_ptr(),
                            title.as_ptr(),
                            0,
                            0,
                            0,
                            1,
                            1,
                            0,
                            0,
                            0,
                            0,
                        )
                    };
                    if hwnd != 0 {
                        focus_hwnd = hwnd;
                        log::info!(
                            target: "verify",
                            "focus_demo hwnd=shell(0x{:x}) (GetConsoleWindow returned NULL)",
                            focus_hwnd
                        );
                    } else {
                        log::warn!(
                            "[playground] GetConsoleWindow + fallback CreateWindowExW both failed"
                        );
                        focus_hwnd = 0;
                    }
                }
                wins[0].set_focus_hwnd(focus_hwnd);
                log::info!(
                    target: "verify",
                    "focus_cmd target=0x{:x}",
                    focus_hwnd
                );
            }

            log::info!(
                target: "verify",
                "sink_demo_attached id={} half_w_phys={:.0} wins=1 scale={:.3}",
                s.id,
                half,
                s.scale
            );
            // rect 心跳：脚本据此判定「块动没动/尺寸变没变」——这是路由→会话→状态
            // 链路的行为证据；像素侧由 cmp_visual/verify_dirty 的零差异门禁兜底。
            let w2 = wins[0].clone();
            std::thread::spawn(move || loop {
                if let Some(s) = w2.snapshot() {
                    log::info!(
                        target: "verify",
                        "sink_demo_state id={} rect=[{:.1},{:.1},{:.1},{:.1}] scale={:.3} visible={}",
                        s.id,
                        s.rect[0],
                        s.rect[1],
                        s.rect[2],
                        s.rect[3],
                        s.scale,
                        s.visible
                    );
                }
                std::thread::sleep(Duration::from_millis(400));
            });
        } else {
            log::warn!("[playground] GLASS_INPUT_SINK_DEMO: 默认块 snapshot 不可用，不挂载");
        }
    }

    // 内容 demo（P0-1 取证入口）：GLASS_CONTENT_DEMO 选择确定性场景，仅构造期读一次
    // （playground 范围；库的 env 例外清单不变）。场景时序由后台线程推流，稳态帧
    // 交给 GLASS_INTERNAL_SHOT_AT_MS=2500 取证；tools/verify_content.py 解析
    // [verify] content_rect / stack_ / dirty_ 日志行定位采样点。
    // 内容 demo 的分发排在 panel 之前：panel::start 会占掉渲染线程头几帧，
    // verify_content 的稳态帧窗口（2.5s）必须全部留给推流 + 交错重绘。
    if let Some(demo) = std::env::var_os("GLASS_CONTENT_DEMO") {
        let mode = demo.to_string_lossy().to_lowercase();
        if mode != "off" && mode != "0" && !mode.is_empty() {
            run_content_demo(&engine, &mode);
        }
    }

    let panel = match panel::start(Arc::clone(&engine), opts.port, opts.no_browser) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("[playground] 控制面板启动失败: {e}（引擎继续运行）");
            None
        }
    };

    // 主线程：把事件打到 stdout（也演示了 poll_events 这条消费路径）。
    let started = Instant::now();
    while engine.is_running() {
        for ev in engine.poll_events() {
            match &ev {
                // Ready / MonitorsChanged / 采集后端 这几条库自己已经打过日志，不重复刷屏
                // FR1 宿主侧焦点：库在按压 Consumed 时发 FocusHint；宿主在
                // **本线程（主线程 = 壳窗口拥有线程）**调 SetFocus——同线程调用
                // 合法且必成。库侧永不触碰焦点（attach 手法实测挂死渲染线程，
                // 见 api/event.rs FocusHint 文档）。
                EngineEvent::FocusHint { id, hwnd } => {
                    println!("[event] FocusHint id={} hwnd=0x{:x}", id, hwnd);
                    if std::env::var_os("GLASS_FOCUS_DEMO").is_some() {
                        let ret = unsafe { SetFocus(*hwnd) };
                        log::info!(
                            target: "verify",
                            "focus_applied id={} target=0x{:x} ret=0x{:x}",
                            id, hwnd, ret
                        );
                    }
                }
                EngineEvent::Ready { .. }
                | EngineEvent::MonitorsChanged { .. }
                | EngineEvent::CaptureBackendSelected { .. } => {}
                other => println!("[event] {other:?}"),
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    println!(
        "[playground] 引擎已退出，运行 {:.1} 秒",
        started.elapsed().as_secs_f64()
    );
    if let Some(p) = panel {
        p.stop();
    }
    // Arc 归零时 Engine::drop 会 join 渲染线程并回收画布窗口。
    match Arc::try_unwrap(engine) {
        Ok(mut e) => e.shutdown(),
        Err(_) => std::thread::sleep(Duration::from_millis(100)),
    }
    ExitCode::SUCCESS
}

struct Opts {
    port: u16,
    no_browser: bool,
    transparent: bool,
    demo_pair: bool,
    skip_default_window: bool,
    level: WindowLevel,
}

fn parse_args() -> Opts {
    let mut o = Opts {
        port: 8787,
        no_browser: false,
        transparent: false,
        demo_pair: true,
        skip_default_window: false,
        level: WindowLevel::default(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--port" => o.port = it.next().and_then(|v| v.parse().ok()).unwrap_or(o.port),
            "--no-browser" => o.no_browser = true,
            "--transparent" => o.transparent = true,
            "--no-demo-pair" => o.demo_pair = false,
            "--no-default-window" => o.skip_default_window = true,
            "--level" => {
                o.level = match it.next().as_deref() {
                    Some("top") => WindowLevel::AlwaysOnTop,
                    Some("normal") => WindowLevel::Normal,
                    Some("click") => WindowLevel::ClickToFront,
                    Some("bottom") => WindowLevel::AlwaysOnBottom,
                    other => {
                        eprintln!(
                            "[playground] 未知 --level {other:?}，用 top/normal/click/bottom"
                        );
                        WindowLevel::default()
                    }
                }
            }
            "--help" | "-h" => {
                println!(
                    "liquid-verre-rs playground\n\n  --port <n>          面板端口（默认 8787）\n  --no-browser        不自动打开浏览器\n  --transparent       默认样式用透明玻璃\n  --no-demo-pair      不额外创建 liquid/transparent 对照窗口\n  --no-default-window 不创建居中默认玻璃\n  --level <top|normal|click|bottom>\n                      画布层级（默认 top）"
                );
            }
            other => eprintln!("[playground] 忽略未知参数 {other}"),
        }
    }
    o
}

/// dirty 场景的子区（纹理空间物理像素，半开区间）。所有 resize 前的默认块
/// 尺寸下限是 720x480（见 runtime seed），子区 256x256 恒在界内。
const VC_DIRTY_RECT: [u32; 4] = [100, 100, 356, 356];
/// dirty 子区重绘颜色（premul BGRA8；红通道 220，与 checker 两档灰都不同）。
const VC_DIRTY_COLOR: [u8; 4] = [40, 40, 220, 255];

fn run_content_demo(engine: &Arc<Engine>, mode: &str) {
    wait_for_monitors(engine);
    let wins = engine.windows();
    if wins.len() != 1 {
        log::warn!(
            "[playground] GLASS_CONTENT_DEMO={mode} needs the single default block (wins={}), skipping",
            wins.len()
        );
        return;
    }
    let lower = wins[0].clone();
    let Some(s) = lower.snapshot() else {
        log::warn!("[playground] GLASS_CONTENT_DEMO={mode}: snapshot unavailable, skipping");
        return;
    };
    log_content_rect(mode, "lower", &s);
    let sc = s.scale.max(0.05);
    let (w0, h0) = (
        (s.rect[2] * sc).round().max(1.0) as u32,
        (s.rect[3] * sc).round().max(1.0) as u32,
    );

    match mode {
        "checker" => {
            lower.update_content(w0, h0, &make_checker(w0, h0), None);
            ready_after_frames(mode, lower.clone());
        }
        "solid" => {
            lower.update_content(w0, h0, &make_solid(w0, h0, [64, 128, 255, 255]), None);
            ready_after_frames(mode, lower.clone());
        }
        "dirty" | "dirty_none" => {
            lower.update_content(w0, h0, &make_checker(w0, h0), None);
            let w = lower.clone();
            let mode_owned = mode.to_string();
            let use_dirty = mode == "dirty";
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(1200));
                let buf = make_checker_dirty(w0, h0);
                let dirty = if use_dirty { Some(VC_DIRTY_RECT) } else { None };
                w.update_content(w0, h0, &buf, dirty);
                log::info!(
                    target: "verify",
                    "dirty_push rect=[{},{},{},{}] color=({},{},{},{}) full_upload={}",
                    VC_DIRTY_RECT[0],
                    VC_DIRTY_RECT[1],
                    VC_DIRTY_RECT[2],
                    VC_DIRTY_RECT[3],
                    VC_DIRTY_COLOR[0],
                    VC_DIRTY_COLOR[1],
                    VC_DIRTY_COLOR[2],
                    VC_DIRTY_COLOR[3],
                    !use_dirty
                );
                ready_after_frames(&mode_owned, w.clone());
            });
        }
        "clear_seq" => {
            lower.update_content(w0, h0, &make_checker(w0, h0), None);
            let w = lower.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(1200));
                w.clear_content();
                log::info!(target: "verify", "clear_push done");
                ready_after_frames("clear_seq", w.clone());
            });
        }
        "stack" | "stack_bare" => {
            // 上块与下块 50% 偏移、同尺寸：重叠区 = 下块右下 1/4（逻辑坐标）。
            let rect = [
                s.rect[0] + s.rect[2] * 0.5,
                s.rect[1] + s.rect[3] * 0.5,
                s.rect[2],
                s.rect[3],
            ];
            let _upper = engine.create_window(WindowConfig {
                rect,
                ..Default::default()
            });
            // 等渲染线程把第 2 块的 snapshot 发布出来（4 Hz 兜底 + dirty 即时）。
            let mut up_snap = None;
            for _ in 0..40 {
                if let Some(u) = engine
                    .windows()
                    .iter()
                    .find(|w| w.id() != lower.id())
                    .and_then(|w| w.snapshot())
                {
                    up_snap = Some(u);
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let Some(u) = up_snap else {
                log::warn!("[playground] stack demo: upper snapshot not published");
                return;
            };
            log_content_rect(mode, "upper", &u);
            if mode == "stack" {
                lower.update_content(w0, h0, &make_checker(w0, h0), None);
            }
            ready_after_frames(mode, lower.clone());
        }
        "resize_seq" | "resize_direct" | "resize_noclear" => {
            // 放大 25%/20%，并把矩形收进本屏可用区（set_size 锚左上角向右下长）。
            let (mon_w, mon_h) = engine
                .monitors()
                .first()
                .map(|m| (m.size[0] as f32 / sc, m.size[1] as f32 / sc))
                .unwrap_or((1920.0, 1080.0));
            let nw = (s.rect[2] * 1.25).min(mon_w - s.rect[0] - 8.0);
            let nh = (s.rect[3] * 1.2).min(mon_h - s.rect[1] - 8.0);
            let (w1, h1) = (
                (nw * sc).round().max(1.0) as u32,
                (nh * sc).round().max(1.0) as u32,
            );
            let w = lower.clone();
            let old_push = mode != "resize_direct";
            let clear_mid = mode == "resize_seq";
            let label = mode.to_string();
            std::thread::spawn(move || {
                if old_push {
                    std::thread::sleep(Duration::from_millis(300));
                    w.update_content(w0, h0, &make_checker(w0, h0), None);
                }
                std::thread::sleep(Duration::from_millis(500));
                w.set_size(nw, nh);
                // resize 后的新块几何 = 取证断言 5 的采样锚点，等快照发布再打日志。
                std::thread::sleep(Duration::from_millis(300));
                if let Some(s2) = w.snapshot() {
                    log_content_rect(&label, "lower_after", &s2);
                }
                if clear_mid {
                    std::thread::sleep(Duration::from_millis(100));
                    w.clear_content();
                }
                std::thread::sleep(Duration::from_millis(400));
                w.update_content(w1, h1, &make_checker(w1, h1), None);
                log::info!(target: "verify", "resize_push new_tex={w1}x{h1} old_push={old_push} cleared={clear_mid}");
                ready_after_frames(&label, w.clone());
            });
        }
        other => {
            log::warn!("[playground] GLASS_CONTENT_DEMO={other}: unknown mode, skipping");
        }
    }
}

/// 块矩形的画布局部物理像素日志——verify_content.py 的采样锚点单一出口。
fn log_content_rect(mode: &str, tag: &str, s: &liquid_verre::api::WindowSnapshot) {
    // 取证 hermetic：GLASS_MONITORS=1 且主屏原点恒 (0,0)，画布局部 = 虚拟屏物理像素。
    let sc = s.scale.max(0.05);
    let x0 = (s.rect[0] * sc).round();
    let y0 = (s.rect[1] * sc).round();
    let x1 = ((s.rect[0] + s.rect[2]) * sc).round();
    let y1 = ((s.rect[1] + s.rect[3]) * sc).round();
    log::info!(
        target: "verify",
        "content_rect mode={mode} tag={tag} id={} rect=[{x0:.0},{y0:.0},{x1:.0},{y1:.0}] scale={sc:.3} radius={:.0}",
        s.id,
        s.style.corner_radius() * sc
    );
}

/// 推流完成后，再让渲染线程跑几帧（UpdateContent/ClearContent 各消费一帧），
/// 然后打 `steady_ready` —— verify_content.py 以此行为准触发稳态帧截图，
/// 避免「shot 帧早于内容入队」的竞态（2026-10-26 bring-up 实测：静态桌面下
/// 首帧之后渲染门控会跳过，shot 必须挂在内容更新触发的那一帧上）。
fn ready_after_frames(mode: &str, w: liquid_verre::api::GlassWindow) {
    let m = mode.to_string();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        log::info!(target: "verify", "steady_ready mode={m}");
        // 心跳：每 150ms 发一条 no-op SetVisible 触发渲染帧（push() 自带
        // request_render），保证稳态帧 deadline 到点时有帧可拍、且内容已入 store。
        // 没有心跳时静态桌面下渲染门控会跳过 shot 之后的所有帧（2026-10-26）。
        for _ in 0..30 {
            w.set_visible(true);
            std::thread::sleep(Duration::from_millis(150));
        }
    });
}

/// dirty 场景用的 checker：在 VC_DIRTY_RECT 内涂 VC_DIRTY_COLOR。
fn make_checker_dirty(w: u32, h: u32) -> Vec<u8> {
    let mut buf = make_checker(w, h);
    let [x0, y0, x1, y1] = VC_DIRTY_RECT;
    for y in y0..y1.min(h) {
        for x in x0..x1.min(w) {
            let (y, x) = (y as usize, x as usize);
            let i = (y * w as usize + x) * 4;
            buf[i..i + 4].copy_from_slice(&VC_DIRTY_COLOR);
        }
    }
    buf
}

/// 脚本是否已经通过 GLASS_* 指定了确定性布局。
/// Generate a deterministic 16x16 checkerboard pattern (BGRA8, pre-multiplied).
fn make_checker(w: u32, h: u32) -> Vec<u8> {
    let size = (w as usize) * (h as usize) * 4;
    let mut buf = vec![0u8; size];
    let tile = 16usize;
    for y in 0..(h as usize) {
        for x in 0..(w as usize) {
            let idx = (y * (w as usize) + x) * 4;
            let black = ((x / tile) + (y / tile)) % 2 == 0;
            if black {
                buf[idx] = 30;
                buf[idx + 1] = 30;
                buf[idx + 2] = 30;
                buf[idx + 3] = 255;
            } else {
                buf[idx] = 220;
                buf[idx + 1] = 220;
                buf[idx + 2] = 220;
                buf[idx + 3] = 255;
            }
        }
    }
    buf
}

/// Generate a solid color fill (BGRA8, pre-multiplied).
fn make_solid(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let size = (w as usize) * (h as usize) * 4;
    let mut buf = vec![0u8; size];
    // Pre-multiply: for opaque alpha=255, color stays as-is.
    let a = rgba[3] as f32 / 255.0;
    let b = (rgba[0] as f32 * a) as u8;
    let g = (rgba[1] as f32 * a) as u8;
    let r = (rgba[2] as f32 * a) as u8;
    for i in (0..size).step_by(4) {
        buf[i] = b;
        buf[i + 1] = g;
        buf[i + 2] = r;
        buf[i + 3] = rgba[3];
    }
    buf
}

fn layout_from_env() -> bool {
    [
        "GLASS_MANY",
        "GLASS_FAR_GRID",
        "GLASS_SPREAD",
        "GLASS_DEFAULT_BLOCK",
    ]
    .iter()
    .any(|k| std::env::var(k).ok().as_deref() == Some("1"))
}

/// 是否处于取证 / 计量模式（落盘截图、打 `[metrics]`、注入确定性负载）。
///
/// 这些模式下场景必须与历史基线**逐块一致**，因此一律不加演示窗口。
fn diagnostics_from_env() -> bool {
    [
        "GLASS_INTERNAL_SHOT",
        "GLASS_SHOT_EVERY_MS",
        "GLASS_METRICS",
        "GLASS_SIM_BACKDROP",
        "GLASS_VERIFY_PASS_A",
        "GLASS_VERIFY_VALIDITY",
    ]
    .iter()
    .any(|k| std::env::var(k).is_ok())
}

/// 等渲染线程发布第一份显示器快照（最多 3 秒）。
fn wait_for_monitors(engine: &Engine) {
    for _ in 0..60 {
        if !engine.monitors().is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    eprintln!("[playground] 等待显示器快照超时，按兜底尺寸摆放演示窗口");
}

/// 主屏物理尺寸（引擎还没发布快照时给一个保守默认值）。
fn primary_size(engine: &Engine) -> [f32; 2] {
    engine
        .monitors()
        .first()
        .map(|m| [m.size[0] as f32, m.size[1] as f32])
        .unwrap_or([1920.0, 1080.0])
}
