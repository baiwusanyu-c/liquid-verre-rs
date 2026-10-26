//! 引擎主循环：消息泵 + 采集调度 + 脏区门控 + 事件驱动渲染。
//!
//! 逐帧顺序（方案 10.4 / 12.4.1，**不可随意调换**，详见
//! `.trellis/spec/backend/rendering-pipeline.md`）：
//!
//! ```text
//! PeekMessage 泵
//! → 消费上层命令（一帧一次）
//! → 每屏采集 try_capture(wait)      # DDA 的阻塞等待同时是主循环唯一的休眠点
//! → prep：默认参数 + 质量档、块几何/投影、窗口区域、点击穿透、影响盒
//! → 渲染门控：触发源全空则整帧跳过（不 Clear、不 Pass A/B、不 Present、不 Commit）
//! → 每屏：整屏 Clear → Pass A（按脏区 scissor，全透明屏整条跳过）→ Pass B（z 序画全部块）
//! → Present(1) → DComp Commit（每屏一次）
//! → QualityCtrl.tick(own_us)        # own_us = prep + A + B + C，**不含**采集等待与 Present 阻塞
//! ```

pub(crate) mod app;
pub(crate) mod commands;
pub(crate) mod monitor;
pub(crate) mod occlusion;

use std::sync::Arc;
use std::time::Instant;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::api::config::EngineConfig;
use crate::api::event::EngineEvent;
use crate::api::handle::Shared;
use crate::capture::Capture;
use crate::core::handle::ResizeEdges;
use crate::core::manager::Manager;
use crate::core::quality::QualityCtrl;
use crate::core::style::StyleCache;
use crate::diag::env::EnvOverrides;
use crate::diag::metrics::{report_metrics, Metrics};
use crate::error::{Error, ErrorKind, Result};
use crate::gfx::{Device, GlassRenderer};
use crate::input::DragSession;
use crate::platform::{display, window};

use app::{
    collect_monitor_blocks, publish_snapshot, rebuild_monitors, render_monitor, spawn_block,
    sync_block_monitor, wait_for_captures, App, FrameCtx, SpawnSpec,
};
use monitor::MonitorRt;

/// 静止时的等待步长：8 ms ≈ 125 Hz，覆盖 120 Hz 显示器的一个刷新周期，
/// 保证鼠标悬停反馈延迟不超过半帧，同时静止桌面 CPU 趋近 0。
const IDLE_WAIT_MS: u32 = 8;

/// 渲染线程入口。返回 `Err` 表示不可恢复的初始化失败。
pub(crate) fn run(shared: Arc<Shared>) -> Result<()> {
    let cfg: EngineConfig = shared.config().clone();
    let env = EnvOverrides::load();

    unsafe {
        if let Err(e) = RoInitialize(RO_INIT_MULTITHREADED) {
            // RPC_E_CHANGED_MODE = 0x80010106：宿主已经用别的模式初始化过，可继续。
            if e.code().0 as u32 != 0x8001_0106 {
                return Err(Error::with_source(
                    ErrorKind::Internal,
                    "WinRT 初始化失败",
                    e,
                ));
            }
        }
        if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).is_err() {
            // DPI 感知是**进程级**的：宿主已经设过别的级别时这里会失败。
            // 不致命，但玻璃的 DPI 换算会不准 —— 必须让上层看见这条告警。
            log::warn!(
                "SetProcessDpiAwarenessContext(PerMonitorV2) 失败：宿主进程已设置过 DPI 感知级别，\
                 多屏不同缩放时玻璃尺寸可能不准。建议宿主在 main() 最前面自行设置为 Per-Monitor V2。"
            );
        }
    }

    let infos = display::enumerate();
    if infos.is_empty() {
        return Err(Error::new(
            ErrorKind::MonitorEnumeration,
            "未枚举到任何显示器",
        ));
    }
    log::info!(
        "显示器: {}",
        infos
            .iter()
            .map(|m| format!(
                "{} {}x{}@({},{}) dpi={:.0} scale={:.2}{}",
                m.name,
                m.size[0],
                m.size[1],
                m.origin[0],
                m.origin[1],
                m.dpi,
                m.scale,
                if m.primary { " [主屏]" } else { "" }
            ))
            .collect::<Vec<_>>()
            .join(" | ")
    );

    // 窗口类注册一次即可；句柄与类名由 platform::window 内部单一来源持有，
    // App 不再各存一份（存了也没人读，只会变成两处可能漂移的副本）。
    let _ = window::register_class(window::module_instance())?;

    // 启动阶段进度日志：驱动异常时 D3D11CreateDevice / D3DCompile 可能长时间无响应，
    // 没有这些打点就无法判断卡在哪一步。
    let t_boot = Instant::now();
    log::info!(target: "boot", "创建 D3D11 设备...");
    let gfx = Device::create().map_err(|e| e.with_kind(ErrorKind::GraphicsInit))?;
    log::info!(
        target: "boot",
        "D3D11 设备就绪 (+{:.0} ms)",
        t_boot.elapsed().as_secs_f64() * 1000.0
    );
    if gfx.warp {
        log::warn!("回退到 WARP 软件光栅");
    }
    log::info!(target: "boot", "编译着色器...");
    let glass =
        GlassRenderer::new(&gfx.device).map_err(|e| e.with_kind(ErrorKind::ShaderCompile))?;
    log::info!(
        target: "boot",
        "着色器就绪 (+{:.0} ms)",
        t_boot.elapsed().as_secs_f64() * 1000.0
    );

    log::info!(target: "boot", "创建各屏画布与采集...");
    let mut monitors_rt: Vec<MonitorRt> = Vec::new();
    for info in infos {
        if let Some(mut m) = MonitorRt::create(&gfx, info, cfg.window_level, cfg.capture.backend) {
            if let Some(cap) = m.capture.as_ref() {
                shared.emit(EngineEvent::CaptureBackendSelected {
                    monitor_index: monitors_rt.len(),
                    monitor_name: m.info.name.clone(),
                    backend: cap.backend_name().to_string(),
                });
                log::info!("{} 采集后端 = {}", m.info.name, cap.backend_name());
            }
            if let Err(e) = m.compositor.resize(&gfx, m.info.size[0], m.info.size[1]) {
                log::warn!("{} 金字塔初始化失败: {e}", m.info.name);
            }
            // 取证图按屏分开，便于逐屏核对多屏渲染结果。
            if let Some(base) = &env.internal_shot {
                m.compositor.shot_path = Some(format!("{base}.m{}.bmp", monitors_rt.len()));
            }
            monitors_rt.push(m);
        }
    }
    if monitors_rt.is_empty() {
        return Err(Error::new(
            ErrorKind::WindowCreation,
            "没有任何显示器能创建画布",
        ));
    }
    // GLASS_MONITORS=<n> / EngineConfig.monitor_limit：只用前 n 块屏。
    // 既是实用选项（把玻璃限制在主屏），也用于归因「第二块屏的采集/合成占多少」。
    if let Some(n) = env.monitor_limit.or(cfg.monitor_limit) {
        if n >= 1 && n < monitors_rt.len() {
            log::info!("只用前 {n} 块屏（monitor_limit）");
            monitors_rt.truncate(n);
        }
    }

    let hz = display::refresh_hz();
    let mut quality = QualityCtrl::new(hz);
    quality.set_fps_cap(cfg.fps_cap);
    // adaptive_quality 是产品正向开关，diagnostics.no_adaptive / GLASS_ADAPTIVE=0
    // 是取证后门，三者任一关闭都禁用（独立审查 P2-1：该字段此前无读取点）。
    if !cfg.adaptive_quality || cfg.diagnostics.no_adaptive || env.no_adaptive {
        quality.disable();
    }
    log::info!(
        "显示器刷新率 = {hz:.0} Hz，电源 = {}，自适应质量 = {}",
        if display::on_battery() {
            "电池"
        } else {
            "交流电"
        },
        if quality.enabled() {
            "开启（默认；level 0 = 与未启用时逐位一致，持续丢帧才降档）"
        } else {
            "关闭"
        }
    );

    // 遮挡剔除（B1）的生效判据在**构造期**算一次：热路径绝不读环境变量（spec 硬规矩）。
    // 产品正向开关 `EngineConfig::occlusion_culling` 与取证后门 `GLASS_OCCLUSION=0`
    // 任一关闭都禁用，与上面 `adaptive_quality` / `GLASS_ADAPTIVE=0` 的组合完全同构。
    let occlusion_enabled = cfg.occlusion_culling && !env.no_occlusion;

    // 「独占全屏应用检测」（方案 R-16 + 0.4.2 的停产门 A）的生效判据同样在**构造期**算一次：
    // 产品开关 `EngineConfig::fullscreen_hide` 与取证后门 `GLASS_FS_HIDE=0` 取「与」，
    // 与上面 `occlusion_culling` / `GLASS_OCCLUSION=0` 的组合完全同构（R-E3）。
    // 为什么必须有取证后门：它的形状判据（WS_POPUP + 无 caption/thickframe + 铺满屏）
    // 与「无边框最大化的普通应用」（Electron / Tauri）完全重合，而停产门 A 把命中后果
    // 从「画布隐藏」升级成了「整屏停产」—— 于是 tools/ 的取证脚本会在「前台恰好是
    // 这类应用」时拿到 0 帧连拍并整批 FAIL（实测 repro_tremble far/many 与 verify_adaptive，
    // 且与 GLASS_OCCLUSION 无关）。闸门必须对「用户前台开着什么」保持 hermetic，
    // 与 GLASS_ADAPTIVE=0 同理。产品代码不要关它：玻璃真被盖住时停产正是收益来源。
    let fs_hide_enabled = cfg.fullscreen_hide && !env.no_fs_hide;

    let mut app = App {
        gfx,
        glass,
        monitors: monitors_rt,
        manager: Manager::default(),
        shared: Arc::clone(&shared),
        default_style: cfg.default_style,
        default_params: cfg.default_style.to_params(),
        level: cfg.window_level,
        default_resize: cfg.default_resize,
        drag: None,
        min_drag_size: [24.0, 24.0],
        hover: None,
        hover_handle: ResizeEdges::NONE,
        cursor: [0.0, 0.0],
        pinned_cursor: env.cursor_at,
        press: 0.0,
        route_state: crate::core::routing::RouteState::Idle,
        drag_velocity: [0.0, 0.0],
        rebuild_monitors: false,
        shutdown: false,
        geometry_changed: false,
        fs_check_at: Instant::now()
            .checked_sub(std::time::Duration::from_secs(3600))
            .unwrap_or_else(Instant::now),
        // 遮挡轮询从「启动后第一个 125 ms」开始：开机这一刻桌面上有什么窗口还没稳定
        // （画布刚建、上层可能还没 create_window），立刻扫一次纯属浪费。
        occl_check_at: Instant::now(),
        occluders: Vec::new(),
        occlusion_enabled,
        occl_paused_mask: 0,
        t0: Instant::now(),
        frames: 0,
        quality,
        backend_pref: cfg.capture.backend,
        style_cache: StyleCache::default(),
        block_sinks: Vec::new(),
        block_focus_hwnds: Vec::new(),
        focus_guard_warned: false,
        content_store: crate::gfx::ContentStore::default(),
        cmd_buf: Vec::new(),
        snapshot: Default::default(),
        monitor_rects: Vec::new(),
        snapshot_key: Vec::new(),
        snapshot_key_next: Vec::new(),
        snapshot_dirty: true,
    };
    app.default_params.sanitize();
    app.refresh_monitor_rects();

    // 光标初值：GLASS_CURSOR_AT 钉死（回归测试用），否则主屏正中。
    app.cursor = app.pinned_cursor.unwrap_or_else(|| {
        let info = &app.monitors[0].info;
        [
            info.origin[0] as f32 + info.size[0] as f32 * 0.5,
            info.origin[1] as f32 + info.size[1] as f32 * 0.5,
        ]
    });

    // 诊断布局：GLASS_MANY / GLASS_FAR_GRID / GLASS_SPREAD（tools/ 的取证脚本依赖）。
    // 正常路径下上层应用通过 Engine::create_window 建窗口，这里一块都不建。
    seed_diagnostic_blocks(&mut app, &env, &cfg);

    // 上层可能在 start() 之前就 create_window，先把这批命令消费掉，
    // 后面的「等采集就绪 → 渲染一帧 → 显示窗口」才不会被黑帧打脸。
    commands::apply(&mut app);
    if env.extreme_spec {
        // 极端回归档：最大高光强度 + 最小锐度，验证 bevel/plane 与圆角转角不断裂。
        app.default_params.spec_intensity = 4.0;
        app.default_params.spec_power = 1.0;
    }

    unsafe {
        app.bind_hwnds();
    }
    for mi in 0..app.monitors.len() {
        collect_monitor_blocks(&mut app, mi);
        let margin = app.monitors[mi].region_margin();
        window::update_region(&app.monitors[mi], margin);
    }

    // overlay 显示前先等到各屏采集就绪，从根源消除初始黑帧。
    wait_for_captures(
        &mut app,
        std::time::Duration::from_millis(u64::from(cfg.capture.startup_wait_ms)),
    );
    // 先渲染/提交一帧完整玻璃，再显示窗口，避免闪黑。
    let initial = FrameCtx {
        cursor_screen: app.cursor,
        prev_cursor_screen: app.cursor,
        drag_velocity: [0.0, 0.0],
        press: 0.0,
        hover_id: None,
        time: app.time(),
    };
    for mi in 0..app.monitors.len() {
        collect_monitor_blocks(&mut app, mi);
        let needs = app.monitors[mi].needs_backdrop;
        if !needs || app.monitors[mi].compositor.capture_ready() {
            let _ = render_monitor(&mut app, mi, &initial, true);
        }
        // 只有拿到真实采集帧的屏才允许显示；全透明屏不依赖采集，直接就绪。
        app.monitors[mi].ready = !needs || app.monitors[mi].compositor.capture_ready();
    }
    for mon in app.monitors.iter() {
        window::apply_visibility(mon);
        if mon.ready {
            log::info!(
                "{} 画布已显示 ({}x{})",
                mon.info.name,
                mon.info.size[0],
                mon.info.size[1]
            );
        }
    }
    let monitor_count = app.monitors.len();
    shared.emit(EngineEvent::Ready { monitor_count });
    publish_snapshot(&mut app, 0.0, hz, display::on_battery());

    // ---- 主循环：消息泵 + 事件驱动渲染（方案 10.4 / 12.4.1）----
    let metrics_enabled = cfg.diagnostics.metrics || env.metrics;
    let sim_busy_us = env.sim_busy_us();
    let verify_shot_path = env.internal_shot.clone();
    let mut verify_done = false;
    let idle_capture_interval =
        std::time::Duration::from_millis(u64::from(cfg.capture.idle_keepalive_ms));
    let retry_backoff = std::time::Duration::from_millis(u64::from(cfg.capture.retry_backoff_ms));
    let mut capture_wait_ms = IDLE_WAIT_MS;
    let mut metrics = Metrics::new();
    let mut sim_next = Instant::now();
    let mut frost_half_applied = false;
    let mut snapshot_at = Instant::now();
    // 采集事件在借用 mon 的循环里产生，先攒着，出循环再发（避免借用冲突）。
    let mut pending_events: Vec<EngineEvent> = Vec::new();
    let mut msg = MSG::default();

    loop {
        let loop_start = Instant::now();
        while unsafe { PeekMessageW(&mut msg, HWND::default(), 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == WM_QUIT {
                app.shutdown = true;
                break;
            }
            let _ = unsafe { TranslateMessage(&msg) };
            unsafe { DispatchMessageW(&msg) };
        }
        if app.shutdown {
            break;
        }
        let msg_us = loop_start.elapsed().as_micros();
        let cap_start = Instant::now();

        if app.rebuild_monitors {
            app.rebuild_monitors = false;
            rebuild_monitors(&mut app);
            app.refresh_monitor_rects();
        }

        // ---- 每屏采集 ----
        // DDA 的阻塞等待同时充当主循环唯一的休眠点（方案 12.4.1「以采集帧为主时钟」）：
        // 桌面有新内容立即返回，静止时在驱动里内核等待。
        let mut new_backdrop = false;
        let mut active = [false; 16];
        let mut wants_bd = [false; 16];
        let mut overflow = false;
        for b in app.manager.blocks() {
            // 读点 1（B1）：被「z 序在画布之上、且不透明」的窗口**完整覆盖**的块，
            // 它的每一个可见像素都被别人挡着 ⇒ 它触发的一切工作都可证明是浪费。
            // 后果链：本屏液态块全被遮挡 ⇒ `wants_bd=false` ⇒ `needs_backdrop_now=false`
            // ⇒ 走全透明屏那条**既有**的停采路径（`idle_capturing`），DDA 次/秒归零。
            // 注意这里只摘「触发源」，不改绘制集合（`mon.blocks` / `cur_rects` 原样），
            // 因此单帧像素输出逐位不变（AC6 用 `GLASS_OCCLUSION=0/1` 同帧对比钉死）。
            if !b.visible || b.occluded {
                continue;
            }
            match active.get_mut(b.monitor) {
                Some(slot) => {
                    *slot = true;
                    if let Some(flag) = wants_bd.get_mut(b.monitor) {
                        // 必须走 effective_style：继承默认样式的块，b.style 是创建时的副本
                        *flag |= b.effective_style(&app.default_style).needs_backdrop();
                    }
                }
                None => overflow = true,
            }
        }
        {
            let gfx = &app.gfx;
            let backend_pref = app.backend_pref;
            for (mi, mon) in app.monitors.iter_mut().enumerate() {
                mon.new_backdrop = false;
                mon.backdrop_dirty = None;
                mon.copied_px = 0;
                // 停产门 A：画布已被独占 / 无边框全屏应用压住（`hidden_fs` ⇒ `SW_HIDE`），
                // 采集与渲染出来的每一个像素都进不了用户眼睛 ⇒ 整屏不采集（R-A1）。
                // 在此之前 `hidden_fs` 只是「可见性位」：DDA 一切正常，于是我们满频跟着
                // 一块看不见的画布渲染 —— 那是纯粹的浪费，不是设计。
                // ⚠ 必须同时置 `idle_capturing`：它不只是「省掉采集」的标记，还是恢复那一帧
                //   `just_activated` 的**判据**。不置位 ⇒ `just_activated` 恒为假 ⇒ 拿不到
                //   「刚恢复所以要阻塞取一帧新桌面」这个结论，玻璃里会是停产前的旧桌面。
                // ⚠ 忙等前科（R-A2）：采集段是主循环**唯一**的休眠点（DDA 在驱动里内核阻塞）。
                //   停产后本屏不再产生 `new_backdrop`；若所有屏都停产则 `need_render=false`，
                //   靠下面 `!need_render` 分支的 `IDLE_WAIT_MS`(8 ms) 速率下限兜住。
                //   少了那道兜底就是 DDA ACCESS_LOST 那次的实测结果：96 kHz / 83% CPU。
                if mon.hidden_fs {
                    mon.idle_capturing = true;
                    continue;
                }
                let has_blocks = overflow || active.get(mi).copied().unwrap_or(true);
                // 全透明屏（或空屏）对桌面内容**零依赖**：连保活采集都不做。
                // 保活的 4Hz `try_capture` 在 DDA 不健康的机器上会让 ReleaseFrame
                // 阻塞到秒级（实测 transparent-drag 场景 cap_max=15989ms、fps 掉到 14），
                // 而那些像素对透明模式毫无用处 —— 停采才是需求 F3.1 的完整形态。
                // 一旦有液态块落回本屏，下面的 `just_activated` 会强制阻塞取一帧新桌面，
                // 不会闪旧背景；金字塔账本由 `just_activated` 那一次 `invalidate()` 兜底 ——
                // 不能只靠 commands 层，因为「遮挡移开」与「全屏应用退出」都不经过 `Command`。
                let needs_backdrop_now = overflow || wants_bd.get(mi).copied().unwrap_or(true);
                if !needs_backdrop_now && mon.capture.is_some() {
                    mon.idle_capturing = true;
                    continue;
                }
                let age = mon.last_capture.elapsed();
                if mon.capture.is_some() && mon.ready && !has_blocks && age < idle_capture_interval
                {
                    continue;
                }
                // 只在「块刚落到这块屏」的那一帧强制阻塞取新帧，避免玻璃里闪出一帧旧桌面。
                // 其余情况：需要背景的屏用主循环决定的等待步长（拖拽时 0，静止时
                // IDLE_WAIT_MS），不需要的屏用 32 ms 保活。
                // 注意不能按「距上次采集多久」判断陈旧 —— 桌面安静时采集本来就稀疏，
                // 那样会让有块的屏也一直走 32 ms 阻塞，白降帧率。
                let just_activated = has_blocks && mon.idle_capturing;
                mon.idle_capturing = !has_blocks;
                if just_activated {
                    // 停产期间桌面可能已经变了，而我们一帧都没采集；金字塔与 present 账本
                    // 却仍声称「这块区域有效」⇒ 恢复后露出陈旧背景。这正是
                    // `verify_validity` 抓的那一类「虚报有效」前科。
                    // 为什么现在才加：透明↔液态切换**必然经过 `Command`**，commands 层的
                    // `invalidate_all` 顺手就兜住了；而 B1 的「遮挡移开」与 A 的
                    // 「全屏应用退出」都**不经过 commands，没有兜底**。
                    // `invalidate()` = `pyramid_valid=None` + `background_valid=false` +
                    // `prev_rects.clear()` + `has_presented=false`，与「采集源重建」
                    // 是同一处理（语义单一来源）。幂等，因此对既有的透明屏→液态屏路径
                    // 只是多一次无害调用；成本是每次「屏从停采恢复」多一次整屏 Pass A，
                    // 而这类翻转本身稀有（跨屏拖拽、建窗口、遮挡移开），恢复帧已经在付
                    // 32 ms 阻塞采集。
                    mon.compositor.invalidate();
                }
                let wait = if just_activated || !needs_backdrop_now {
                    32
                } else {
                    capture_wait_ms
                };
                let (outcome, dirty, copied) = match mon.capture.as_mut() {
                    Some(cap) => {
                        let r = cap.try_capture(&gfx.device, &gfx.ctx, wait);
                        (r, cap.dirty_union(), cap.copied_px())
                    }
                    None => {
                        if Instant::now() >= mon.retry_at {
                            mon.retry_at = Instant::now() + retry_backoff;
                            match Capture::for_monitor_pref(
                                &gfx.device,
                                mon.info.hmonitor,
                                backend_pref,
                            ) {
                                Ok(cap) => {
                                    if mon.capture_error_logged {
                                        log::info!("{} 采集已恢复", mon.info.name);
                                        pending_events.push(EngineEvent::CaptureRestored {
                                            monitor_index: mi,
                                        });
                                    }
                                    mon.capture_error_logged = false;
                                    mon.capture = Some(cap);
                                    // 采集源重建后金字塔与后备缓冲都失效，下一帧必须整屏重来。
                                    mon.compositor.invalidate();
                                }
                                Err(_) => {
                                    if !mon.capture_error_logged {
                                        mon.capture_error_logged = true;
                                        log::warn!(
                                            "{} 采集不可用，{} ms 后退避重试",
                                            mon.info.name,
                                            retry_backoff.as_millis()
                                        );
                                    }
                                }
                            }
                        }
                        continue;
                    }
                };
                match outcome {
                    Ok(Some((tex, w, h, changed))) => {
                        if mon.compositor.width != w || mon.compositor.height != h {
                            if let Err(e) = mon.compositor.resize(gfx, w, h) {
                                log::error!("resize({}): {e}", mon.info.name);
                            }
                        }
                        if let Err(e) = mon.compositor.set_capture(gfx, tex, changed) {
                            log::error!("set_capture({}): {e}", mon.info.name);
                        }
                        mon.new_backdrop = changed;
                        mon.backdrop_dirty = dirty;
                        mon.copied_px = copied;
                        mon.last_capture = Instant::now();
                        if !mon.ready && mon.compositor.capture_ready() {
                            mon.ready = true;
                            mon.hidden_capture = false;
                            window::apply_visibility(mon);
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        // ACCESS_LOST / 锁屏（安全桌面）：释放并安排退避重建，绝不在错误上忙等。
                        // 锁屏时 DDA 会持续返回 E_ACCESSDENIED，解锁后自动恢复，无需人工干预。
                        if !mon.capture_error_logged {
                            mon.capture_error_logged = true;
                            log::warn!(
                                "{} DDA 失效（锁屏/独占全屏/被其他采集占用？）: {e}；每 {} ms 退避重试，恢复后自动继续",
                                mon.info.name,
                                retry_backoff.as_millis()
                            );
                            pending_events.push(EngineEvent::CaptureLost {
                                monitor_index: mi,
                                kind: ErrorKind::CaptureLost,
                                reason: e.to_string(),
                            });
                        }
                        mon.retry_at = Instant::now() + retry_backoff;
                        mon.capture = None;
                        if mon.ready {
                            // 画面会冻结在最后一帧，因此采集一失效就隐藏画布，恢复后再显示。
                            mon.hidden_capture = true;
                            window::apply_visibility(mon);
                        }
                        mon.ready = false;
                    }
                }
                if mon.new_backdrop {
                    new_backdrop = true;
                }
            }
        }
        for ev in pending_events.drain(..) {
            shared.emit(ev);
        }

        let cap_us = cap_start.elapsed().as_micros();
        let prep_start = Instant::now();

        // 确定性负载（GLASS_SIM_BACKDROP）：真实桌面的活动率会在 4~52 次/秒之间漂移，
        // 8 秒的测量窗口根本不可比；只有把负载确定性化，分段归因才有意义。
        if let Some((sw_, sh_, shz)) = env.sim_backdrop {
            let interval = std::time::Duration::from_secs_f64(1.0 / shz);
            let now = Instant::now();
            if now >= sim_next {
                // 脏区固定放在主屏正中，保证每帧 Pass A / Pass B 的工作量完全一致
                let (mo, ms) = {
                    let info = &app.monitors[0].info;
                    (
                        [info.origin[0], info.origin[1]],
                        [info.size[0], info.size[1]],
                    )
                };
                let cx = mo[0] + ms[0] / 2;
                let cy = mo[1] + ms[1] / 2;
                let abs = [
                    (cx - sw_ / 2).max(mo[0]),
                    (cy - sh_ / 2).max(mo[1]),
                    (cx + sw_ / 2).min(mo[0] + ms[0]),
                    (cy + sh_ / 2).min(mo[1] + ms[1]),
                ];
                // 合成器按**画布局部坐标**消费 backdrop_dirty（与 DDA 的
                // dirty_union 同量纲），而上面算的是虚拟屏绝对坐标。主屏原点为
                // (0,0) 时两者恰好重合，所以常规取证没暴露；左屏当副屏的排布下
                // 这份矩形会落到框外 -> run_rect=None -> 确定性负载静默失效
                // （独立审查 P2-7，污染的是 gate_perf 的输入）。
                for m in app.monitors.iter_mut() {
                    let (ox, oy) = (m.info.origin[0], m.info.origin[1]);
                    m.new_backdrop = true;
                    m.backdrop_dirty = Some([abs[0] - ox, abs[1] - oy, abs[2] - ox, abs[3] - oy]);
                }
                new_backdrop = true;
                sim_next += interval;
                if now.duration_since(sim_next) > interval * 4 {
                    sim_next = now + interval;
                }
            } else {
                for m in app.monitors.iter_mut() {
                    m.new_backdrop = false;
                    m.backdrop_dirty = None;
                }
                new_backdrop = false;
            }
        }

        // ---- 上层命令（每帧消费一次，帧首一次性合并）----
        let geometry_from_wndproc = std::mem::take(&mut app.geometry_changed);
        let commands_changed = commands::apply(&mut app);

        // ---- 默认参数 + 自适应质量档 ----
        let previous_params = app.default_params;
        app.default_params = app.default_style.to_params();
        if env.extreme_spec {
            app.default_params.spec_intensity = 4.0;
            app.default_params.spec_power = 1.0;
        }
        let quality_changed = app.quality.take_level_changed();
        app.quality.apply(&mut app.default_params);
        let mut geometry_changed = geometry_from_wndproc;
        let params_changed =
            commands_changed || quality_changed || app.default_params != previous_params;
        // level 1 的 frost 半分辨率档：切换会重建金字塔，只在档位真的变化时做
        let want_half = app.quality.frost_half();
        if want_half != frost_half_applied {
            for mon in app.monitors.iter_mut() {
                if let Err(e) = mon.compositor.set_frost_half(&app.gfx, want_half) {
                    log::warn!("{} 切换 frost 档位失败: {e}", mon.info.name);
                }
            }
            frost_half_applied = want_half;
        }

        // ---- 鼠标状态（每帧轮询，无 hook；方案 6.3）----
        let previous_cursor = app.cursor;
        app.cursor = app.read_cursor();
        let cursor_moved = (app.cursor[0] - previous_cursor[0]).abs() > 0.01
            || (app.cursor[1] - previous_cursor[1]).abs() > 0.01;
        // hover 与「缩放把手」一起判定：把手命中盒伸出块外，因此必须先查把手，
        // 否则块外一点点的边缘反馈会晚一帧才出现。
        let (new_hover, new_handle) = match app.drag.as_ref() {
            Some(d) => (Some(d.id()), d.edges().unwrap_or(ResizeEdges::NONE)),
            None => match app.manager.hit_resize(app.cursor[0], app.cursor[1]) {
                Some((id, e)) => (Some(id), e),
                None => (
                    app.manager.hit_test(app.cursor[0], app.cursor[1]),
                    ResizeEdges::NONE,
                ),
            },
        };
        let hover_changed = new_hover != app.hover || new_handle != app.hover_handle;
        app.hover = new_hover;
        app.hover_handle = new_handle;

        // ---- 拖拽 / 缩放：每帧应用一次（方案 12.2.3 输入合并）----
        // 先算好上下文，避免在 app.drag 的可变借用里再借用 app。
        let drag_id = app.drag.as_ref().map(DragSession::id);
        let drag_scale = drag_id.map_or(1.0, |id| {
            app.scale_of(app.manager.get(id).map_or(0, |b| b.monitor))
        });
        let min_size = app.min_drag_size;
        let drag_monitor = match app.drag.as_mut() {
            Some(session) => {
                let now = app.t0.elapsed().as_secs_f64();
                let id = session.id();
                let resizing = session.is_resize();
                // 用块**当前**所在屏的比例换算；跨屏后 sync_block_monitor 会再校正。
                let scale = drag_scale;
                let r = session.apply(app.cursor, now, scale, min_size);
                app.drag_velocity = session.velocity();
                if let Some(b) = app.manager.get_mut(id) {
                    b.x = r[0];
                    b.y = r[1];
                    // 移动时绝不写回尺寸：尺寸归 DPI 换算管，否则跨屏后会被
                    // 按下那一刻的像素尺寸覆盖（跨屏回归测试抓到的真实缺陷）。
                    if resizing {
                        b.width = r[2] * scale;
                        b.height = r[3] * scale;
                    }
                    b.clamp();
                }
                // 跨屏：中心越过边界就换屏，并按两屏 DPI 比例换算尺寸。
                sync_block_monitor(&mut app, id);
                Some(app.manager.get(id).map_or(0, |b| b.monitor))
            }
            None => {
                app.drag_velocity[0] *= 0.85;
                app.drag_velocity[1] *= 0.85;
                None
            }
        };
        let dragging = drag_monitor.is_some();
        if dragging {
            geometry_changed = true;
        }

        // ---- 每屏：投影块 + 窗口区域 + 点击穿透 ----
        let mut mouse_relevant = false;
        for mi in 0..app.monitors.len() {
            collect_monitor_blocks(&mut app, mi);
            // 空屏画布隐藏（1 秒防抖）：见 MonitorRt::hidden_empty 的说明。
            {
                let empty = app.monitors[mi].blocks.is_empty();
                let mon = &mut app.monitors[mi];
                if empty {
                    match mon.empty_since {
                        Some(t) if t.elapsed() >= std::time::Duration::from_secs(1) => {
                            if !mon.hidden_empty {
                                mon.hidden_empty = true;
                                window::apply_visibility(mon);
                            }
                        }
                        Some(_) => {}
                        None => mon.empty_since = Some(Instant::now()),
                    }
                } else {
                    mon.empty_since = None;
                    if mon.hidden_empty {
                        mon.hidden_empty = false;
                        window::apply_visibility(mon);
                    }
                }
            }
            let mon_dragging = drag_monitor == Some(mi);
            // 拖拽只影响被拖块所在的那块屏；其他屏的区域不必重建
            // （SetWindowRgn 会让 DWM 重裁剪整个 overlay，是长尾的主要来源）。
            let region_idle = dragging && !mon_dragging;
            if mon_dragging {
                if !app.monitors[mi].drag_region_full {
                    window::set_region_full(&app.monitors[mi]);
                    app.monitors[mi].drag_region_full = true;
                }
            } else if !region_idle
                && (app.monitors[mi].drag_region_full
                    || app.monitors[mi].region_dirty
                    || geometry_changed)
            {
                let margin = app.monitors[mi].region_margin();
                window::update_region(&app.monitors[mi], margin);
                app.monitors[mi].drag_region_full = false;
                app.monitors[mi].region_dirty = false;
            }

            // 光标是否可能影响本屏任一块（鼠标点光与缩放反馈都是屏幕空间场）
            let origin = app.monitors[mi].canvas_origin();
            let influence = app.monitors[mi].influence_px() as f32;
            let a = [
                previous_cursor[0] - origin[0],
                previous_cursor[1] - origin[1],
            ];
            let b = [app.cursor[0] - origin[0], app.cursor[1] - origin[1]];
            let x0 = a[0].min(b[0]) - influence;
            let y0 = a[1].min(b[1]) - influence;
            let x1 = a[0].max(b[0]) + influence;
            let y1 = a[1].max(b[1]) + influence;
            let mon = &app.monitors[mi];
            // 读点 3（B1）：被完整遮挡的块不参与光标影响盒判定。
            // 这是 B1 里**单位收益最高**的一处 —— `mouse_relevant` 不只触发整帧重画，
            // 还会把下面的 `capture_wait_ms` 打到 **0**，让采集从「在驱动里内核阻塞」
            // 退化成满频轮询（CPU 11.87% / 16.25% 那一档的成因之一）。
            if cursor_moved
                && mon.blocks.iter().any(|blk| {
                    !blk.occluded
                        && blk.x < x1
                        && blk.x + blk.width > x0
                        && blk.y < y1
                        && blk.y + blk.height > y0
                })
            {
                mouse_relevant = true;
            }

            // 点击穿透：光标在本屏某块内（含缩放命中带）→ 取消穿透；否则恢复（方案 12.1.4）
            let grip = mon.max_grip;
            // interactive=false 的块不参与：它要画出来，但点击必须穿透到下层应用
            let inside = mon_dragging
                || mon.blocks.iter().any(|blk| {
                    blk.interactive
                        && b[0] >= blk.x - grip
                        && b[0] <= blk.x + blk.width + grip
                        && b[1] >= blk.y - grip
                        && b[1] <= blk.y + blk.height + grip
                });
            window::set_click_through(mon.hwnd, inside);
        }

        // ---- 逐块遮挡检测（B1）：8 Hz 轮询，节流在函数内部 ----
        // 放在 `collect_monitor_blocks` 的循环**之后**：它要读本屏刚投影出来的
        // `mon.blocks`（绘制矩形 = 该块可见像素的硬边界）；放在渲染门控**之前**：
        // 本轮结论要能被下一圈的采集需求扫描与 `mouse_relevant` 读到。
        // 因此读点拿到的是上一轮结果，滞后 ≤1 圈（≤8 ms）—— 对 8 Hz 的决策毫无意义。
        occlusion::update(&mut app);

        // 独占全屏应用检测（方案 R-16）：4 Hz 轮询，命中就隐藏该屏画布 + 整屏停产，
        // 避免全屏置顶画布破坏游戏的独占全屏与 MPO。
        // `fs_hide_enabled` 为假时整块跳过（`EngineConfig::fullscreen_hide` 与取证后门
        // GLASS_FS_HIDE=0 取「与」，见上方构造期注释）。
        if fs_hide_enabled && app.fs_check_at.elapsed() >= std::time::Duration::from_millis(250) {
            app.fs_check_at = Instant::now();
            for (mi, mon) in app.monitors.iter_mut().enumerate() {
                // 拖拽中隐藏画布会释放 SetCapture，把拖拽状态机打断，
                // 因此拖拽期间推迟到抬起后再处理。
                // ⚠ 轮询路径**零分配**：`fullscreen_app_on` 只返回命中的 HWND，
                //   身份字符串（`window_desc`）只在下面的翻转沿才构造（R-E1）。
                let matched = if dragging {
                    None
                } else {
                    display::fullscreen_app_on(mon.hwnd)
                };
                let hit = matched.is_some();
                if hit != mon.hidden_fs {
                    mon.hidden_fs = hit;
                    if !hit {
                        // 停产期间桌面变了而我们一帧都没采集：金字塔与 present 账本都不可信。
                        mon.compositor.invalidate();
                        // 需要桌面背景的屏走「采集失效恢复」那条**已验证**的路径：
                        // `ready=false` ⇒ 下面这次 `apply_visibility` 继续 `SW_HIDE`，
                        // 画布保持隐藏；等下一次采集到 `Ok(Some(..))` 时命中既有的
                        // `if !mon.ready && mon.compositor.capture_ready()` 分支自动
                        // `ready=true` + 显示。**零新状态机**。
                        // 代价（诚实记录）：玻璃晚约一帧 + 一次 32 ms 阻塞采集才出现；
                        // 换来「绝不呈现停产前的陈旧桌面」（R-A3 优先于 40 ms 延迟）。
                        // ⚠ `needs_backdrop` 这个条件**不可省**：全透明屏不采样桌面，
                        //   没有「新采集帧」可等 ⇒ 把 `ready` 打成 false 会让它**永远不回来**
                        //   （透明模式本来也没有陈旧问题）。
                        if mon.needs_backdrop {
                            mon.ready = false;
                        }
                    }
                    window::apply_visibility(mon);
                    // 翻转沿日志。`matched` 是**真正触发判定的那个前台窗口**（不是「打日志这
                    // 一刻恰好在前台的窗口」—— 4 Hz 轮询与翻转沿之间前台可以变，记错了比不记更误导）。
                    // `hit == matched.is_some()` 且此刻正处于翻转沿，所以两个分支恰好各对应一次跃迁。
                    // ⚠ 命中侧那句「检测到独占全屏应用，暂时隐藏玻璃」是
                    //   tools/verify_occlusion.py 的 MARKS["fs_on"] **子串匹配**目标，
                    //   原文不可改，窗口身份只能**追加在后面**（改措辞 = 打断取证脚本）。
                    match matched {
                        Some(fg) => {
                            let (class, title, rect) = display::window_desc(fg);
                            log::info!(
                                "{} 检测到独占全屏应用，暂时隐藏玻璃：class={class} title={title:?} rect={rect:?}",
                                mon.info.name
                            );
                        }
                        None => {
                            log::info!("{} 独占全屏应用已退出，恢复玻璃", mon.info.name);
                        }
                    }
                    pending_events.push(EngineEvent::ExclusiveFullscreen {
                        monitor_index: mi,
                        active: hit,
                    });
                }
            }
        }

        // 快照发布：状态真的变了立刻发，否则 4 Hz 兜底（拖拽期靠 dirty 走即时路径）。
        if app.snapshot_dirty || snapshot_at.elapsed() >= std::time::Duration::from_millis(250) {
            snapshot_at = Instant::now();
            let frames = app.frames;
            publish_snapshot(&mut app, frames as f32, hz, display::on_battery());
        }

        let prep_us = prep_start.elapsed().as_micros();

        // ---- 渲染门控（方案 12.4.1）：没有任何触发源就整帧跳过 ----
        let velocity_active =
            app.drag_velocity[0].abs() >= 0.01 || app.drag_velocity[1].abs() >= 0.01;
        let requested = shared.take_render_request();
        let fps_capped = !app.quality.allow_present(Instant::now());
        let need_render = !fps_capped
            && (new_backdrop
                || params_changed
                || geometry_changed
                || hover_changed
                || mouse_relevant
                || dragging
                || velocity_active
                || requested);

        if !need_render {
            // 静止：不重画、不 Present、不 Commit。
            capture_wait_ms = IDLE_WAIT_MS;
            // 速率下限兜底：正常情况下 AcquireNextFrame 已经在驱动里阻塞了 IDLE_WAIT_MS；
            // 一旦采集源失效并立即返回，这里保证主循环绝不会退化成数十 kHz 的空转
            // （实测缺陷：DDA ACCESS_LOST 时 96 kHz / 83% CPU）。
            let elapsed = loop_start.elapsed();
            let floor = std::time::Duration::from_millis(u64::from(IDLE_WAIT_MS));
            if elapsed < floor {
                std::thread::sleep(floor - elapsed);
            }
            if metrics_enabled {
                report_metrics(
                    &mut app,
                    &mut metrics,
                    loop_start.elapsed().as_micros(),
                    0,
                    false,
                    [msg_us, cap_us, prep_us],
                );
            }
            continue;
        }
        capture_wait_ms = if dragging || velocity_active || mouse_relevant {
            0
        } else {
            IDLE_WAIT_MS
        };

        let ctx = FrameCtx {
            cursor_screen: app.cursor,
            prev_cursor_screen: previous_cursor,
            drag_velocity: app.drag_velocity,
            press: app.press,
            hover_id: app.hover,
            time: app.time(),
        };
        let render_start = Instant::now();
        // 只在 GLASS_SIM_BUSY>0 且正在拖拽时存在：模拟「拖拽期我们自己的工作超预算」，
        // 于是 verify_adaptive 的时间线（拖拽 = 重负载 / 松手 = 轻负载）能确定性地
        // 驱动降档与回升，不依赖桌面当时到底忙不忙。
        let mut sim_us: u128 = 0;
        if sim_busy_us > 0 && dragging {
            let spin = Instant::now();
            while spin.elapsed().as_micros() < sim_busy_us {
                std::hint::spin_loop();
            }
            sim_us = spin.elapsed().as_micros();
        }
        let mut presented = false;
        // 判据只能用「我们自己的工作」：渲染前准备 + Pass A + Pass B + Commit。
        // 不能用 render_us / loop_us，因为它们里含两段阻塞等待（详见 QualityCtrl 文档）：
        //   采集阶段等桌面新帧 8~32 ms、Present 等 vblank/DWM 背压 6.6~25 ms。
        // last_stage_us = [Pass A, Pass B, Present, Commit]，这里故意跳过 Present。
        let mut own_us: u128 = prep_us + sim_us;
        for mi in 0..app.monitors.len() {
            // 停产门 A：被全屏应用压住的屏这一帧**根本不参与**（不 Clear、不 Pass A/B、
            // 不 Present、不 Commit）。必须放在 `ready` 判定**之前** —— `hidden_fs` 的屏
            // `ready` 往往仍是 true，只有这一道门挡得住它。GPU 的大头正在这里
            // （实测 baseline 77 fps、skip-PassA→110、skip-PassB→114）。
            // 不调 `render_monitor` ⇒ `presented` 不置位 ⇒ `quality.tick(own_us, false)`
            // 看到的是一个「零负载」窗口，自适应控制器不会因此误降档。
            if app.monitors[mi].hidden_fs {
                continue;
            }
            // 全透明屏不依赖采集，ready 恒为 true；液态屏必须等首帧就绪。
            if !app.monitors[mi].ready && app.monitors[mi].needs_backdrop {
                continue;
            }
            if render_monitor(&mut app, mi, &ctx, false) {
                presented = true;
                let st = app.monitors[mi].compositor.last_stage_us;
                own_us += st[0] + st[1] + st[3];
            }
        }
        let render_us = render_start.elapsed().as_micros();

        // 自证：对每一块屏，用同一张 capture 立刻再渲染一帧「全量 Pass A +
        // 全量 Pass B（整屏清屏 + 全部块重画）」，输出 .full.bmp 对照图。
        // 两张 overlay 必须逐像素一致，否则说明脏区 / 采样余量算漏了。
        if env.verify_pass_a && !verify_done {
            let fired: Vec<usize> = (0..app.monitors.len())
                .filter(|&mi| app.monitors[mi].compositor.shot_fired)
                .collect();
            if !fired.is_empty() {
                if let Some(base) = &verify_shot_path {
                    verify_done = true;
                    for mi in fired {
                        app.monitors[mi].compositor.force_full_next();
                        app.monitors[mi].compositor.shot_path =
                            Some(format!("{base}.m{mi}.full.bmp"));
                        app.monitors[mi].compositor.shot_fired = false;
                        render_monitor(&mut app, mi, &ctx, true);
                        log::info!(target: "verify", "已输出全量对照帧: {base}.m{mi}.full.bmp");
                    }
                }
            }
        }

        if presented {
            app.frames += 1;
            app.quality.note_present(Instant::now());
        }
        if app.quality.tick(own_us, presented) {
            app.snapshot_dirty = true;
            let level = app.quality.level();
            shared.emit(EngineEvent::QualityLevelChanged { level });
        }
        if metrics_enabled {
            report_metrics(
                &mut app,
                &mut metrics,
                loop_start.elapsed().as_micros(),
                render_us,
                presented,
                [msg_us, cap_us, prep_us],
            );
        }
    }

    // ---- 收尾：先解绑 GWLP_USERDATA 再销毁窗口 ----
    // 顺序不能反：App 一旦离开作用域，残留画布的窗口过程就会拿到悬垂指针。
    unsafe {
        app.unbind_hwnds();
    }
    let hwnds: Vec<HWND> = app.monitors.iter().map(|m| m.hwnd).collect();
    app.monitors.clear();
    for hwnd in hwnds {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
    Ok(())
}

/// 诊断布局（`GLASS_MANY` / `GLASS_FAR_GRID` / `GLASS_SPREAD`）。
///
/// `tools/` 下的取证脚本靠这些**确定性布局**做逐位比对，因此必须留在库里；
/// 但正常路径下一块都不建 —— 上层应用自己 `Engine::create_window`。
fn seed_diagnostic_blocks(app: &mut App, env: &EnvOverrides, cfg: &EngineConfig) {
    let [lw, lh] = app.primary_size();
    let style = app.default_style;
    let resize = app.default_resize;
    let spawn = |app: &mut App, rect: [f32; 4]| {
        let id = app.shared.alloc_id();
        spawn_block(app, SpawnSpec::diag(id, rect, style, resize));
    };

    if env.far_grid {
        // GLASS_FAR_GRID=1：4 块互不重叠、分开足够远的确定性布局，
        // 专供 tools/verify_mouse_isolation.py 做「拖一块，其余块逐位不变」的硬证明。
        // 为什么不能直接用 GLASS_MANY：那 20 块是密集重叠布局（步长 0.62w/0.60h），
        // 邻近余晖会沿重叠块的半透明阴影逐块传递，合法余晖与非法串扰无法区分，
        // 判定集合的传递闭包会吞掉全部 20 块，测试就失去判别力。
        // 光标落在块 0 内时，其余块的中心距都 > gate 支撑半径 3*mouse_radius=660，
        // 因此光标落在任一块上时，其余块必须逐位与光标在远处时完全一致。
        let cells: [(f32, f32); 4] = [
            (600.0, 300.0),
            (1800.0, 300.0),
            (600.0, 1300.0),
            (1800.0, 1300.0),
        ];
        for &(bx, by) in &cells {
            let x = bx.min((lw - 500.0).max(8.0));
            let y = by.min((lh - 340.0).max(8.0));
            spawn(app, [x, y, 500.0, 340.0]);
        }
        return;
    }
    if env.spread {
        // GLASS_SPREAD=1：在每块显示器正中各放一块，用于多屏验收。
        // 尺寸以「主屏物理像素」为逻辑单位，落到别的屏时按该屏 DPI 换算，
        // 因此在分辨率 / 缩放各不相同的屏上物理观感一致（方案 12.2.5）。
        for mi in 0..app.monitors.len() {
            let info = app.monitors[mi].info.clone();
            let sc = info.scale.max(0.05);
            let w = 600.0 * sc;
            let h = 400.0 * sc;
            let x = info.origin[0] as f32 + (info.size[0] as f32 - w) * 0.5;
            let y = info.origin[1] as f32 + (info.size[1] as f32 - h) * 0.5;
            let id = app.shared.alloc_id();
            spawn_block(app, SpawnSpec::diag(id, [x, y, w, h], style, resize));
            if let Some(b) = app.manager.get_mut(id) {
                b.monitor = mi;
                b.scale = sc;
            }
        }
        return;
    }
    if env.many {
        // GLASS_MANY=1：20 块密集级联布局（压力测试）。步长与重构前 add_auto 一致，
        // 否则 gate_perf 的 many-drag 场景与历史数字不可比。
        for i in 0..20 {
            let w = 600.0 + (i % 4) as f32 * 30.0;
            let h = 400.0 + (i % 3) as f32 * 24.0;
            let gap = 22.0;
            let step_x = (w * 0.62 + gap).max(96.0);
            let step_y = (h * 0.60 + gap).max(84.0);
            let cols = (((lw - 40.0) / step_x).floor() as usize).clamp(1, 8);
            let col = i % cols;
            let row = (i / cols) % 12;
            let x = (32.0 + col as f32 * step_x).min((lw - w - 24.0).max(8.0));
            let y = (64.0 + row as f32 * step_y).min((lh - h - 32.0).max(8.0));
            spawn(app, [x, y, w, h]);
        }
        return;
    }
    // 一块居中玻璃。**默认不建**：库不替上层决定要不要有窗口。
    // `tools/` 的取证脚本用 GLASS_DEFAULT_BLOCK=1 显式打开（尺寸与重构前一致，
    // 否则历史逐位比对图不可比）；playground 用 EngineConfig 打开。
    if !(env.default_block || cfg.diagnostics.default_window) {
        return;
    }
    let w = (lw * 0.62).clamp(720.0, 1200.0);
    let h = (lh * 0.68).clamp(480.0, 760.0);
    spawn(app, [(lw - w) * 0.5, (lh - h) * 0.5, w, h]);
}

#[cfg(test)]
mod tests {
    use super::IDLE_WAIT_MS;

    #[test]
    fn idle_wait_matches_the_documented_tradeoff() {
        // 8 ms ≈ 125 Hz 轮询：
        //  - 上限：不能超过一个 60 Hz 周期（16.7 ms），否则静止桌面的唤醒频率
        //    低于合成节拍，鼠标悬停反馈会明显迟滞；
        //  - 下限：必须小到能覆盖 120 Hz（8.33 ms）的一个刷新周期，
        //    这样高刷屏上悬停反馈的延迟不超过半帧。
        let ms = f64::from(IDLE_WAIT_MS);
        assert!(ms <= 16.7, "静止唤醒步长不能超过一个 60 Hz 周期");
        assert!(
            ms * 1.0 <= 1000.0 / 120.0 + 0.5,
            "必须覆盖 120 Hz 的一个刷新周期"
        );
        assert!(ms >= 4.0, "再小就退化成忙等，静止 CPU 降不下去");
    }
}
