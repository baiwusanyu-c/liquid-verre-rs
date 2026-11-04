//! 0.5.2 宿主接入契约的集成测试（**不启动渲染线程**，无显示器 / 无 GPU 也能跑）。
//!
//! `api_surface.rs` 管「公共面存在且能序列化」，这里管「宿主据以写绕行代码的那几条
//! 行为承诺」：每一条都对应 0.5.2 修复清单里宿主实测过的一个缺陷，改坏任何一条
//! 都应当在这里红，而不是等宿主集成时才发现。
//!
//! 需要真实渲染的验收在 `tools/` 下：
//! `verify_border_zorder.py`（#1 像素 oracle）、`verify_session_snapshot.py`（#4）、
//! `verify_double_click.py`（#5）。

use liquid_verre::{
    Engine, EngineConfig, GlassStyle, GlassWindow, MonitorSnapshot, PointerPhase,
    ResizeFeedbackOptions, ResizeOptions, WindowConfig, WindowLevel, WindowSnapshot,
    MIN_FLOOR_SUSPICIOUS,
};

fn engine() -> Engine {
    Engine::new(EngineConfig::default()).expect("构造不应依赖显示器")
}

/// #8：`frame_latency` 必须是**配置**入口（宿主按产品场景调参），
/// 而不是只有环境变量；默认 `None` 保证不改任何既有基线。
#[test]
fn frame_latency_is_a_config_knob_with_unchanged_default() {
    let e = engine();
    assert_eq!(
        e.config().frame_latency,
        None,
        "默认必须是 None（= 队列深度 1，跟手优先），否则 0.5.2 会悄悄改滞后基线"
    );
    let mut cfg = EngineConfig {
        frame_latency: Some(2),
        ..Default::default()
    };
    cfg.sanitize();
    assert_eq!(cfg.frame_latency, Some(2));
    let json = serde_json::to_string(&cfg).unwrap();
    assert!(json.contains("\"frameLatency\":2"), "camelCase: {json}");
    let back: EngineConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(back.frame_latency, Some(2));
}

/// #8：越界值夹紧到 [1, 8]，与 `gfx/device.rs` 实际下发给 DXGI 的口径同一来源。
#[test]
fn frame_latency_clamps_into_the_dxgi_sane_band() {
    for (raw, want) in [(Some(0), 1u32), (Some(1), 1), (Some(8), 8), (Some(99), 8)] {
        let mut cfg = EngineConfig {
            frame_latency: raw,
            ..Default::default()
        };
        cfg.sanitize();
        assert_eq!(cfg.frame_latency, Some(want), "raw={raw:?}");
    }
}

/// #7：`min_width/min_height` 是**硬下限**，库里不偷偷改小。
///
/// 这条是「宿主必须自己夹」这个约定的回归网：如果哪天库里加了「相对块尺寸的
/// 上限」而改变了语义，本测试会红，文档也得跟着改（见 `api/config.rs` 模块头）。
#[test]
fn size_floor_is_a_hard_floor_and_only_warns() {
    let host_floor = MIN_FLOOR_SUSPICIOUS + 100.0;
    let mut r = ResizeOptions {
        min_width: host_floor,
        min_height: host_floor,
        ..Default::default()
    };
    r.sanitize();
    assert_eq!(
        (r.min_width, r.min_height),
        (host_floor, host_floor),
        "可疑下限只报警、不改数值（改了就不是「上层说什么就是什么」了）"
    );

    // 默认值必须远在可疑线以下：否则「装完就报警」本身就是缺陷。
    let d = ResizeOptions::default();
    assert!(d.min_width < MIN_FLOOR_SUSPICIOUS && d.min_height < MIN_FLOOR_SUSPICIOUS);
}

/// #7：把手带 / border 宽度的量纲同样是逻辑像素（= 主屏物理像素），
/// `sanitize` 的区间不许把宿主按 DPI 放大后的值吃掉。
#[test]
fn grip_and_border_width_survive_a_dpi_scaled_value() {
    // 200% 主屏上宿主按 DPI 放大：grip 10 -> 20，border 5 -> 10。
    let mut r = ResizeOptions {
        grip_px: 20.0,
        feedback: ResizeFeedbackOptions {
            border_width: 10.0,
            ..Default::default()
        },
        ..Default::default()
    };
    r.sanitize();
    assert_eq!(r.grip_px, 20.0, "DPI 放大后的把手带必须原样生效");
    assert_eq!(r.feedback.border_width, 10.0);
}

/// #5：`PointerPhase::DoubleClick` 是公共面的一部分（库侧唯一的双击出口）。
///
/// 类样式少了 `CS_DBLCLKS` 时这个变体**永不产生**（`platform/window.rs` 有类级
/// 单测 + `tools/verify_double_click.py` 有物理取证），这里钉住「宿主可以依赖它」。
#[test]
fn double_click_phase_is_part_of_the_public_surface() {
    let phases = [
        PointerPhase::Down,
        PointerPhase::Up(None),
        PointerPhase::Move,
        PointerPhase::DoubleClick,
        PointerPhase::RightDown,
        PointerPhase::Wheel,
    ];
    assert_eq!(
        phases
            .iter()
            .filter(|p| **p == PointerPhase::DoubleClick)
            .count(),
        1
    );
    assert_ne!(PointerPhase::DoubleClick, PointerPhase::Down);
}

/// #3：`canvas_handle` 是宿主压底 / OLE 重挂的唯一抓手，必须在快照里恒可用。
#[test]
fn monitor_snapshot_exposes_the_canvas_handle_field() {
    let e = engine();
    let snaps: Vec<MonitorSnapshot> = e.monitors();
    assert!(
        snaps.is_empty(),
        "未 start 时不该有显示器快照，但字段必须存在（编译期即契约）"
    );
    // 字段可读且类型稳定：宿主拿它直接喂 SetWindowPos。
    let probe = MonitorSnapshot {
        name: "probe".to_string(),
        origin: [0, 0],
        size: [1920, 1080],
        dpi: 96.0,
        scale: 1.0,
        primary: true,
        capture_backend: "none".to_string(),
        canvas_visible: false,
        canvas_handle: 0x1234,
    };
    assert_eq!(probe.canvas_handle, 0x1234);
}

/// #2 / #3：`AlwaysOnBottom` 是宿主分类框的推荐层级；`set_window_level` 等值调用
/// 必须是**无害**的：不 panic、不返回错误、不因为「值没变」而在库内被丢掉
/// （丢掉 = 宿主拿不到任何「重新压底」的能力，只能自己发 SetWindowPos）。
///
/// 注意 `Engine::new` 之后渲染线程还没起，命令只是入队，所以这里断言的是
/// 「调用被接受 + 快照尚未被改写」，而不是层级真的变了。
#[test]
fn setting_the_same_level_twice_is_accepted_and_idempotent() {
    let e = engine();
    for _ in 0..3 {
        e.set_window_level(WindowLevel::AlwaysOnBottom);
    }
    assert_eq!(
        EngineConfig::default().window_level,
        WindowLevel::AlwaysOnTop,
        "默认层级不许被 0.5.2 改动（基线承诺）"
    );
    let snap = e.snapshot();
    assert_eq!(
        snap.window_level,
        WindowLevel::AlwaysOnTop,
        "未启动时快照仍是初始值（命令异步，宿主不该指望同步生效）"
    );
    assert!(
        e.poll_events().is_empty(),
        "未启动不该有事件（等值重发不许在入队阶段就制造事件）"
    );
}

/// 0.5.3 #9：hover **归属**是宿主可读的公共面（`WindowSnapshot::hovered`），
/// 而且 #9/#10 都**不新增公共 API** —— 宿主本轮不需要改代码这条承诺的前提就是它。
///
/// 判据本体（`topmost_body_z_at` / `grip_reachable` / `hover_gate_blocks`）是 `pub(crate)`，
/// 在 `core/manager.rs` 与 `runtime::mod` 里各有单测；这里只钉「宿主读得到结果」。
#[test]
fn hover_attribution_is_observable_and_needs_no_new_api() {
    let e = engine();
    let w = e
        .create_window(WindowConfig {
            rect: [100.0, 100.0, 400.0, 300.0],
            ..Default::default()
        })
        .expect("合法配置");
    // 未启动 ⇒ 没有任何权威状态，也就**不该**有虚假的 hover（宿主的「当前操作框」UI
    // 若读到 hovered=true 会亮出一个根本不存在的悬停态）。
    assert!(w.snapshot().is_none(), "引擎未启动时不该有快照");
    let s = e.snapshot();
    assert!(
        s.windows.iter().all(|win| !win.hovered),
        "未启动时 hovered 必须全 false"
    );
    // 编译期契约：宿主诊断通路（debug_log_blocks 每秒打 hovered=）依赖这个字段。
    fn hover_field_is_public(win: &WindowSnapshot) -> bool {
        win.hovered
    }
    let reads: Vec<bool> = s.windows.iter().map(hover_field_is_public).collect();
    assert!(reads.is_empty() || reads.iter().all(|v| !*v));
}

/// #4：宿主跟随窗口位置/尺寸靠的是 `WindowSnapshot::rect` + `moving`/`resizing`
/// 三个字段。它们必须一直在（缺字段时宿主的 16 ms tick 绕行无处可写）。
#[test]
fn snapshot_exposes_geometry_and_session_flags_too() {
    let e = engine();
    let w: GlassWindow = e
        .create_window(WindowConfig {
            rect: [100.0, 100.0, 400.0, 300.0],
            style: Some(GlassStyle::default()),
            ..Default::default()
        })
        .expect("合法配置");
    assert!(w.snapshot().is_none(), "引擎未启动时还没有权威状态");
    let s = e.snapshot();
    // 契约靠类型检查钉住：宿主跟随几何要用的三个字段必须一直在公共面上。
    fn follow_fields(win: &WindowSnapshot) -> (bool, bool, [f32; 4]) {
        (win.moving, win.resizing, win.rect)
    }
    let reads: Vec<_> = s.windows.iter().map(follow_fields).collect();
    assert!(
        reads.is_empty(),
        "引擎未启动时快照里没有窗口（渲染线程还没发布过）"
    );
}
