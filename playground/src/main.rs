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
    DiagnosticsConfig, Engine, EngineConfig, EngineEvent, GlassStyle, ResizeOptions, WindowConfig,
    WindowLevel,
};

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

/// 脚本是否已经通过 GLASS_* 指定了确定性布局。
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
