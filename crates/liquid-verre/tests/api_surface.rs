//! 公共 API 面的集成测试。
//!
//! 这里**不启动渲染线程**（集成测试环境可能没有显示器 / 桌面会话），
//! 只验证「上层应用不碰 GPU 就能依赖的契约」：配置序列化、错误类型、
//! 句柄语义、构建元数据。需要真实渲染的验收走 `tools/` 下的取证脚本。

use liquid_verre::{
    BuildInfo, DiagnosticsConfig, Engine, EngineConfig, ErrorKind, GlassMode, GlassStyle,
    LiquidParams, ResizeOptions, TransparentParams, WindowConfig, WindowLevel, ZOrder, BUILD_INFO,
};

fn cfg() -> EngineConfig {
    EngineConfig::default()
}

#[test]
fn engine_can_be_constructed_without_a_display() {
    // Engine::new 只做配置校验与日志安装，不碰 D3D11 —— 上层可以在没有桌面的
    // 环境（服务、CI）里构造并序列化配置。
    let engine = Engine::new(cfg()).expect("构造不应依赖显示器");
    assert!(!engine.is_running());
    assert!(engine.monitors().is_empty(), "未 start 时不应有显示器快照");
    assert!(engine.windows().is_empty());
}

#[test]
fn create_window_validates_before_touching_gpu() {
    let engine = Engine::new(cfg()).unwrap();
    assert_eq!(
        engine
            .create_window(WindowConfig {
                rect: [0.0, 0.0, 0.0, 100.0],
                ..Default::default()
            })
            .expect_err("零宽必须被拒")
            .kind(),
        ErrorKind::InvalidConfig
    );
    assert_eq!(
        engine
            .create_window(WindowConfig {
                rect: [0.0, 0.0, f32::NAN, 100.0],
                ..Default::default()
            })
            .expect_err("NaN 必须被拒")
            .kind(),
        ErrorKind::InvalidConfig
    );
    // 合法配置在 start 之前也能拿到句柄（命令会在第一帧被消费）
    let w = engine
        .create_window(WindowConfig {
            rect: [10.0, 20.0, 300.0, 200.0],
            ..Default::default()
        })
        .expect("合法配置必须能建句柄");
    assert!(!w.is_alive(), "引擎未启动时窗口还不存在");
    assert_eq!(w.rect(), None);
}

#[test]
fn handles_are_cheap_clones_and_compare_by_identity() {
    let engine = Engine::new(cfg()).unwrap();
    let a = engine.create_window(WindowConfig::default()).unwrap();
    let b = a.clone();
    assert_eq!(a, b, "同一窗口的句柄克隆必须相等");
    let c = engine.create_window(WindowConfig::default()).unwrap();
    assert_ne!(a, c);
    assert_eq!(
        engine.window(a.id()),
        a,
        "engine.window(id) 必须取回等价句柄"
    );
}

#[test]
fn setters_on_dead_windows_are_silent_noops() {
    let engine = Engine::new(cfg()).unwrap();
    let w = engine.window(4242);
    // 不 panic、不返回 Err：上层持有旧句柄是常态（窗口可能被别处关掉）
    w.set_rect([0.0, 0.0, 100.0, 100.0]);
    w.set_size(50.0, 50.0);
    w.set_position(1.0, 2.0);
    w.set_style(Some(GlassStyle::default()));
    w.set_liquid(None);
    w.set_transparent(10.0, 0.5, [1.0; 4]);
    w.set_corner_radius(20.0);
    w.set_opacity(0.3);
    w.set_color([0.0; 4]);
    w.set_resize_options(ResizeOptions::default());
    w.set_resizable(false);
    w.set_z_order(ZOrder::Front);
    w.bring_to_front();
    w.send_to_back();
    w.set_visible(false);
    w.set_interactive(false);
    w.close();
    assert_eq!(w.snapshot(), None);
    assert_eq!(w.canvas_handle(), None);
    assert_eq!(w.monitor_index(), None);
    assert_eq!(w.mode(), None);
}

#[test]
fn config_serde_roundtrip_covers_every_public_type() {
    let c = EngineConfig {
        default_style: GlassStyle::Transparent(TransparentParams {
            corner_radius: 21.0,
            opacity: 0.42,
            color: [0.1, 0.2, 0.3, 0.4],
        }),
        window_level: WindowLevel::ClickToFront,
        default_resize: ResizeOptions {
            grip_px: 17.0,
            ..Default::default()
        },
        diagnostics: DiagnosticsConfig {
            default_window: true,
            metrics: true,
            ..Default::default()
        },
        fps_cap: Some(90),
        monitor_limit: Some(1),
        ..Default::default()
    };
    let json = serde_json::to_string_pretty(&c).expect("serialize");
    let back: EngineConfig = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, c);
    // camelCase 是跨语言契约（上层可能是 JS / C# / Python）
    assert!(json.contains("\"windowLevel\""), "{json}");
    assert!(json.contains("\"defaultStyle\""), "{json}");
    assert!(json.contains("\"gripPx\""), "{json}");

    let w = WindowConfig {
        rect: [1.0, 2.0, 3.0, 4.0],
        style: Some(GlassStyle::Liquid(LiquidParams {
            ior: 1.42,
            ..Default::default()
        })),
        ..Default::default()
    };
    let wj = serde_json::to_string(&w).unwrap();
    let wb: WindowConfig = serde_json::from_str(&wj).unwrap();
    assert_eq!(wb, w);
    match wb.style {
        Some(GlassStyle::Liquid(p)) => assert!((p.ior - 1.42).abs() < 1e-6),
        other => panic!("模式往返失败: {other:?}"),
    }
}

#[test]
fn partial_json_uses_defaults_for_missing_fields() {
    // 上层只传它关心的字段是常态（TOML/JSON 配置文件）
    let c: EngineConfig = serde_json::from_str(r#"{"windowLevel":"alwaysOnBottom"}"#).unwrap();
    assert_eq!(c.window_level, WindowLevel::AlwaysOnBottom);
    assert_eq!(
        c.default_style.mode(),
        GlassMode::Liquid,
        "缺省必须回到默认模式"
    );
    assert!(c.adaptive_quality, "自适应默认开启");
    let w: WindowConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(w, WindowConfig::default());
}

#[test]
fn mode_switch_is_expressible_purely_through_config() {
    // 需求 F3.4：模式切换必须能只靠配置表达，不需要重建窗口
    let liquid = GlassStyle::default();
    let transparent = GlassStyle::transparent(16.0, 0.5, [1.0, 1.0, 1.0, 1.0]);
    assert_eq!(liquid.mode(), GlassMode::Liquid);
    assert_eq!(transparent.mode(), GlassMode::Transparent);
    assert!(liquid.needs_backdrop());
    assert!(!transparent.needs_backdrop());
    // 透明模式的参数面必须只有三项
    let t: TransparentParams =
        serde_json::from_str(r#"{"cornerRadius":12.0,"opacity":0.7,"color":[1,1,1,1]}"#).unwrap();
    assert_eq!(t.corner_radius, 12.0);
    assert_eq!(t.opacity, 0.7);
}

#[test]
fn build_info_is_populated_and_consistent() {
    let info: &BuildInfo = &BUILD_INFO;
    assert!(!info.version.is_empty());
    assert!(!info.target_triple.is_empty());
    assert!(info.shaders.iter().any(|(n, _)| *n == "glass_ps.hlsl"));
    assert!(
        info.shaders.iter().any(|(n, _)| *n == "border_ps.hlsl"),
        "新增的 border 着色器必须进构建摘要: {:?}",
        info.shaders
    );
    let s = info.summary();
    assert!(s.starts_with("liquid-verre-rs "), "{s}");
}

#[test]
fn error_display_and_kind_are_stable_for_matching() {
    // 上层按 kind 分支，不匹配字符串 —— 这里钉住 Display 的格式契约
    let e = liquid_verre::Error::new(ErrorKind::CaptureUnavailable, "DDA 与 WGC 都拿不到");
    assert_eq!(e.to_string(), "[capture-unavailable] DDA 与 WGC 都拿不到");
    assert_eq!(e.kind(), ErrorKind::CaptureUnavailable);
    // std::error::Error 兼容：可以被 anyhow/thiserror 风格的上层捕获
    let boxed: Box<dyn std::error::Error> = Box::new(e);
    assert!(boxed.to_string().contains("capture-unavailable"));
}

#[test]
fn shutdown_before_start_is_safe() {
    let mut engine = Engine::new(cfg()).unwrap();
    engine.shutdown();
    engine.shutdown();
    assert!(!engine.is_running());
    assert!(engine.take_fatal_error().is_none());
}

#[test]
fn poll_events_without_running_engine_is_empty() {
    let engine = Engine::new(cfg()).unwrap();
    assert!(engine.poll_events().is_empty());
    assert_eq!(engine.quality_level(), 0);
    assert!(engine.snapshot().windows.is_empty());
}

#[test]
#[ignore = "需要真实桌面会话：cargo test --release -p liquid-verre-rs -- --ignored"]
fn two_round_engine_lifecycle_does_not_fail_on_class_already_exists() {
    // 修复 #2 的集成验证体：同进程两轮 Engine::new + start + shutdown
    // 第二轮不得报 ErrorKind::WindowCreation（RegisterClassExW “类已存在”应被容忍）。
    // start 需要显示器；无显示器时直接 skip（不 panic）。

    use liquid_verre::{Engine, EngineConfig, ErrorKind};

    let config = EngineConfig::default();

    // 第一轮
    let mut engine = match Engine::new(config.clone()) {
        Ok(e) => e,
        Err(e) => {
            if e.kind() == ErrorKind::MonitorEnumeration {
                eprintln!("无显示器，跳过集成测试");
                return;
            }
            panic!("第一轮 Engine::new 失败: {e}");
        }
    };
    match engine.start() {
        Ok(()) => {}
        Err(e) => {
            if e.kind() == ErrorKind::MonitorEnumeration {
                eprintln!("无显示器，跳过集成测试");
                return;
            }
            panic!("第一轮 start 失败: {e}");
        }
    };
    engine.shutdown();
    // 等待渲染线程结束
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(!engine.is_running(), "shutdown 后 is_running 应为 false");
    assert!(engine.take_fatal_error().is_none(), "第一轮不应有致命错误");

    // 第二轮：不得因 RegisterClassExW “类已存在”而报 WindowCreation
    let mut engine2 = match Engine::new(config) {
        Ok(e) => e,
        Err(e) => {
            if e.kind() == ErrorKind::MonitorEnumeration {
                eprintln!("无显示器，跳过集成测试");
                return;
            }
            panic!("第二轮 Engine::new 失败: {e}");
        }
    };
    match engine2.start() {
        Ok(()) => {}
        Err(e) => {
            assert_ne!(
                e.kind(),
                ErrorKind::WindowCreation,
                "第二轮不得因 RegisterClassExW “类已存在”而报 WindowCreation: {e}"
            );
            if e.kind() == ErrorKind::MonitorEnumeration {
                eprintln!("无显示器，跳过集成测试");
                return;
            }
            panic!("第二轮 start 失败: {e}");
        }
    };
    engine2.shutdown();
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(!engine2.is_running());
    assert!(engine2.take_fatal_error().is_none(), "第二轮不应有致命错误");
}
