#![windows_subsystem = "console"]
//! liquid-glass —— Windows 桌面液态玻璃（严格按《折射渲染方案》v1.1 实现）
//!
//! 管线（**每个显示器一套**，方案 4.1 / 12.10）：
//!   DXGI Desktop Duplication 采集该显示器 -> 1 份共享 D3D11 纹理（脏矩形局部 copy）
//!   -> 该屏的 dual-filter 模糊金字塔（只刷新「块足迹 ∩ 桌面脏区」）
//!   -> 该屏一个全屏画布 + DirectComposition（每块 scissor 绘制，一次 Present / Commit）
//!   -> Snell 折射 + RGB 色散 + Fresnel 高光 + 焦散 + 鼠标点光 + 自适应染色
//!
//! 性能设计（方案第 7 / 12 章）：
//!   - 采集拷贝成本 O(桌面脏区)，与块数无关；
//!   - 模糊金字塔 O(块足迹 ∩ 脏区)，与块数无关；
//!   - 块绘制只覆盖脏区内的像素；
//!   - 单设备、每屏一个交换链与一次 Present，20+ 块不增加合成开销；
//!   - 事件驱动：无触发源时整帧跳过，静止桌面不 Present、不 Commit。
//!
//! 多屏（需求 4）：每屏独立采集纹理与金字塔，分辨率 / DPI 各不相同也各自正确；
//! 块几何以主屏物理像素为逻辑单位，跨屏时按 DPI 比例换算（方案 12.2.5），
//! 并默认钳制在单屏内（方案 12.10 策略 ①）。

mod capture;
mod gfx;
mod glass;
mod input;
mod ui;

use std::time::Instant;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, UpdateWindow, GDI_REGION_TYPE,
    RGN_OR,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::*;

use capture::DdaCapture;
use gfx::{monitors, Compositor, Device, GlassRenderer, MonitorInfo};
use glass::{Block, Manager, Params};
use input::{cursor_pos, DragKind, DragState};

/// 窗口过程里发生的、必须触发一帧重画的状态变化（按下/抬起/快捷键/显示变更）。
static RENDER_REQUEST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn request_render() {
    RENDER_REQUEST.store(true, std::sync::atomic::Ordering::Release);
}

/// 一个显示器 = 一个全屏画布窗口 + 一个交换链 + 一个 DDA 采集 + 一套金字塔。
/// 各屏分辨率 / DPI 完全独立，互不影响。
struct MonitorRt {
    info: MonitorInfo,
    hwnd: HWND,
    compositor: Compositor,
    capture: Option<DdaCapture>,
    /// 首帧采集是否已就绪（未就绪的屏不显示，避免黑玻璃）
    ready: bool,
    region_dirty: bool,
    drag_region_full: bool,
    /// 本屏块的**画布局部坐标**副本（复用，热路径零分配）
    blocks: Vec<Block>,
    order: Vec<usize>,
    /// 按本屏 DPI 缩放后的参数
    params: Params,
    new_backdrop: bool,
    backdrop_dirty: Option<[i32; 4]>,
    copied_px: i64,
    /// 采集失效后的退避重建时间点（方案 4.3）
    retry_at: Instant,
    /// 失败日志只在状态翻转时打一次：锁屏期间每 500 ms 重试一次，
    /// 否则会刷出成千上万行相同告警。
    capture_error_logged: bool,
    /// 本屏的缩放参数是否需要重算（避免每帧 clone + sanitize 一遍 Params）
    params_dirty: bool,
}

impl MonitorRt {
    fn canvas_origin(&self) -> [f32; 2] {
        [self.info.origin[0] as f32, self.info.origin[1] as f32]
    }

    fn create(
        gfx: &Device,
        class_name: PCWSTR,
        hinst: HINSTANCE,
        info: MonitorInfo,
    ) -> Option<Self> {
        let name = info.name.clone();
        let hwnd: HWND = unsafe {
            CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW,
                class_name,
                windows::core::w!("liquid glass canvas"),
                WS_POPUP,
                info.origin[0],
                info.origin[1],
                info.size[0],
                info.size[1],
                HWND::default(),
                None,
                hinst,
                None,
            )
        }
        .unwrap_or_default();
        if hwnd.0.is_null() {
            eprintln!("[warn] {name} 画布窗口创建失败");
            return None;
        }
        unsafe {
            // 让 DComp 接管整窗（无重定向位图）
            let m = MARGINS {
                cxLeftWidth: -1,
                cxRightWidth: -1,
                cyTopHeight: -1,
                cyBottomHeight: -1,
            };
            let _ = DwmExtendFrameIntoClientArea(hwnd, &m);
            // 自反馈根治：把该画布从所有采集（DDA / WGC / PrintWindow）中剔除。
            const WDA_EXCLUDEFROMCAPTURE: u32 = 0x0000_0011;
            if let Err(e) =
                SetWindowDisplayAffinity(hwnd, WINDOW_DISPLAY_AFFINITY(WDA_EXCLUDEFROMCAPTURE))
            {
                eprintln!("[warn] SetWindowDisplayAffinity({name}): {e:?}");
            }
        }
        let compositor = match Compositor::new(gfx, hwnd, info.size[0], info.size[1]) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[warn] {name} 合成器初始化失败: {e}");
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
                return None;
            }
        };
        let capture = match DdaCapture::new_for_monitor(&gfx.device, info.hmonitor) {
            Ok(c) => Some(c),
            Err(e) => {
                // 该屏可能挂在另一块显卡上，或被别的录屏软件占用了 duplication。
                eprintln!("[warn] {name} 采集不可用，跳过该屏: {e}");
                None
            }
        };
        Some(Self {
            info,
            hwnd,
            compositor,
            capture,
            ready: false,
            region_dirty: true,
            drag_region_full: false,
            blocks: Vec::new(),
            order: Vec::new(),
            params: Params::default(),
            new_backdrop: false,
            backdrop_dirty: None,
            copied_px: 0,
            retry_at: Instant::now(),
            capture_error_logged: false,
            params_dirty: true,
        })
    }

    fn show(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            let _ = UpdateWindow(self.hwnd);
        }
    }
}

/// C 模式：所有块画进所在显示器的全屏画布，各屏共享一份着色器与 LUT。
struct App {
    gfx: Device,
    glass: GlassRenderer,
    monitors: Vec<MonitorRt>,
    manager: Manager,
    params: Params,
    shared: &'static ui::SharedParams,
    hinst: HINSTANCE,
    class_name: PCWSTR,
    last_block_action: isize,
    drag: Option<DragState>,
    hover: Option<u32>,
    /// 光标位置：虚拟屏物理像素（绝对坐标，多屏通用）
    cursor: [f32; 2],
    press: f32,
    drag_velocity: [f32; 2],
    rebuild_monitors: bool,
    t0: Instant,
    frames: u64,
}

/// 一帧的全局输入（各屏共用，坐标为虚拟屏绝对物理像素）。
#[derive(Clone, Copy)]
struct FrameCtx {
    cursor_screen: [f32; 2],
    prev_cursor_screen: [f32; 2],
    drag_velocity: [f32; 2],
    press: f32,
    hover_id: Option<u32>,
    time: f32,
}

impl App {
    fn time(&self) -> f32 {
        self.t0.elapsed().as_secs_f32()
    }

    fn primary_size(&self) -> [f32; 2] {
        match self.monitors.first() {
            Some(m) => [m.info.size[0] as f32, m.info.size[1] as f32],
            None => [1920.0, 1080.0],
        }
    }

    fn scale_of(&self, monitor: usize) -> f32 {
        self.monitors
            .get(monitor)
            .map_or(1.0, |m| m.info.scale.max(0.05))
    }

    fn monitor_index_at(&self, x: f32, y: f32) -> usize {
        let infos: Vec<MonitorInfo> = self.monitors.iter().map(|m| m.info.clone()).collect();
        monitors::index_at(&infos, x, y).min(self.monitors.len().saturating_sub(1))
    }
}

/// 把块归位到它中心所在的显示器，并按 DPI 比例换算尺寸（方案 12.2.5 / 12.10 ①）。
fn sync_block_monitor(app: &mut App, id: u32) {
    let Some(b) = app.manager.get(id) else {
        return;
    };
    let (cx, cy) = (b.x + b.width * 0.5, b.y + b.height * 0.5);
    let target = app.monitor_index_at(cx, cy);
    let (old_scale, w, h, r) = (b.scale, b.width, b.height, b.radius);
    let new_scale = app.scale_of(target);
    let Some(b) = app.manager.get_mut(id) else {
        return;
    };
    if target != b.monitor || (new_scale - old_scale).abs() > 1e-4 {
        // 逻辑尺寸不变，物理像素尺寸按两屏 DPI 比例换算，观感保持一致。
        let k = new_scale / old_scale.max(1e-3);
        b.width = w * k;
        b.height = h * k;
        b.radius = r * k;
        b.scale = new_scale;
        b.monitor = target;
    }
    // 钳制：块中心必须留在本屏内（默认不做跨屏大玻璃，方案 12.10 策略 ①）。
    let info = &app.monitors[b.monitor].info;
    let (ox, oy) = (info.origin[0] as f32, info.origin[1] as f32);
    let (sw, sh) = (info.size[0] as f32, info.size[1] as f32);
    b.x = b.x.clamp(ox - b.width * 0.5, ox + sw - b.width * 0.5);
    b.y = b.y.clamp(oy - b.height * 0.5, oy + sh - b.height * 0.5);
    b.clamp();
}

fn sync_selection(app: &App) {
    let selected = app
        .shared
        .selected_id
        .load(std::sync::atomic::Ordering::Relaxed);
    let id = (selected >= 0)
        .then_some(selected as u32)
        .filter(|id| app.manager.get(*id).is_some())
        .or_else(|| app.manager.blocks().last().map(|b| b.id));
    let value = id.map_or(-1, |id| id as isize);
    app.shared
        .selected_id
        .store(value, std::sync::atomic::Ordering::Relaxed);
    if let Some(id) = id {
        if let Some(block) = app.manager.get(id) {
            let s = block.scale.max(1e-3);
            app.shared.new_width.store(
                (block.width / s).round() as isize,
                std::sync::atomic::Ordering::Relaxed,
            );
            app.shared.new_height.store(
                (block.height / s).round() as isize,
                std::sync::atomic::Ordering::Relaxed,
            );
        }
    }
}

/// 取与窗口关联的 App；未设置时返回 None
unsafe fn app_of(hwnd: HWND) -> Option<&'static mut App> {
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if raw == 0 {
        None
    } else {
        Some(&mut *(raw as *mut App))
    }
}

extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_LBUTTONDOWN => {
                if let Some(app) = app_of(hwnd) {
                    let p = cursor_pos();
                    let c = [p.x as f32, p.y as f32];
                    if let Some(id) = app.manager.hit_test(c[0], c[1]) {
                        app.manager.bring_to_front(id, &app.params);
                        app.shared
                            .selected_id
                            .store(id as isize, std::sync::atomic::Ordering::Relaxed);
                        let (gw, gh, grip, origin, phys) = match app.manager.get(id) {
                            Some(b) => {
                                let s = b.scale.max(1e-3);
                                (
                                    (b.width / s).round() as isize,
                                    (b.height / s).round() as isize,
                                    22.0 * b.scale,
                                    // origin 的 w/h 存**逻辑单位**，跨屏拖拽/缩放
                                    // 才不会被按下那一刻的像素尺寸锁死。
                                    [b.x, b.y, b.width / s, b.height / s],
                                    [b.width, b.height],
                                )
                            }
                            None => (600, 400, 22.0, [0.0; 4], [0.0; 2]),
                        };
                        app.shared
                            .new_width
                            .store(gw, std::sync::atomic::Ordering::Relaxed);
                        app.shared
                            .new_height
                            .store(gh, std::sync::atomic::Ordering::Relaxed);
                        let near_corner = c[0] > origin[0] + phys[0] - grip
                            && c[1] > origin[1] + phys[1] - grip;
                        let kind = if near_corner {
                            DragKind::ResizeSE
                        } else {
                            DragKind::Move
                        };
                        app.drag = Some(DragState::begin(
                            id,
                            kind,
                            c,
                            origin,
                            app.t0.elapsed().as_secs_f64(),
                        ));
                        app.press = 1.0;
                        let _ = SetCapture(hwnd);
                        request_render();
                    }
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                if let Some(app) = app_of(hwnd) {
                    let p = cursor_pos();
                    app.cursor = [p.x as f32, p.y as f32];
                    app.hover = app.manager.hit_test(app.cursor[0], app.cursor[1]);
                }
                LRESULT(0)
            }
            WM_NCHITTEST => {
                if let Some(app) = app_of(hwnd) {
                    let p = cursor_pos();
                    let c = [p.x as f32, p.y as f32];
                    if app.drag.is_some() || app.manager.hit_test(c[0], c[1]).is_some() {
                        return LRESULT(HTCLIENT as isize);
                    }
                    return LRESULT(HTTRANSPARENT as isize);
                }
                LRESULT(HTTRANSPARENT as isize)
            }
            WM_LBUTTONUP => {
                if let Some(app) = app_of(hwnd) {
                    app.press = 0.0;
                    if let Some(d) = app.drag.take() {
                        if let Some(b) = app.manager.get(d.id) {
                            let s = b.scale.max(1e-3);
                            app.shared.new_width.store(
                                (b.width / s).round() as isize,
                                std::sync::atomic::Ordering::Relaxed,
                            );
                            app.shared.new_height.store(
                                (b.height / s).round() as isize,
                                std::sync::atomic::Ordering::Relaxed,
                            );
                            app.shared
                                .selected_id
                                .store(d.id as isize, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                }
                let _ = ReleaseCapture();
                request_render();
                LRESULT(0)
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED => {
                // 分辨率 / 缩放 / 拔插显示器：重建每屏画布与采集。
                if let Some(app) = app_of(hwnd) {
                    app.rebuild_monitors = true;
                }
                request_render();
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            WM_KEYDOWN => {
                request_render();
                if wp.0 == 0x1B {
                    PostQuitMessage(0);
                    return LRESULT(0);
                }
                if let Some(app) = app_of(hwnd) {
                    // 键盘快捷：+ / - 增删一块（无控制面板时的最小可用路径）
                    match wp.0 as u32 {
                        0xBB => {
                            let i = app.manager.len();
                            let size = app.primary_size();
                            let id = app.manager.add_auto(i, 320.0, 200.0, size[0], size[1]);
                            sync_block_monitor(app, id);
                            for m in app.monitors.iter_mut() {
                                m.region_dirty = true;
                            }
                            sync_selection(app);
                        }
                        0xBD => {
                            let id = app.manager.blocks().last().map(|b| b.id);
                            if let Some(id) = id {
                                app.manager.remove(id);
                                for m in app.monitors.iter_mut() {
                                    m.region_dirty = true;
                                }
                            }
                            sync_selection(app);
                        }
                        _ => {}
                    }
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

/// 把块投影到某块屏的画布局部坐标（含按 DPI 缩放的参数），并缓存 z 序。
fn collect_monitor_blocks(app: &mut App, mi: usize, refresh_params: bool) {
    let (ox, oy, scale) = {
        let info = &app.monitors[mi].info;
        (
            info.origin[0] as f32,
            info.origin[1] as f32,
            info.scale.max(0.05),
        )
    };
    let mon = &mut app.monitors[mi];
    if refresh_params || mon.params_dirty {
        mon.params = app.params.scaled(scale);
        mon.params_dirty = false;
    }
    mon.blocks.clear();
    for b in app.manager.blocks() {
        if b.monitor != mi {
            continue;
        }
        let mut local = b.clone();
        local.x = b.x - ox;
        local.y = b.y - oy;
        mon.blocks.push(local);
    }
    mon.blocks.sort_by_key(|b| b.z_order);
    mon.order.clear();
    mon.order.extend(0..mon.blocks.len());
}

/// Restrict the canvas to the union of the blocks and their shadow margins.
/// SetWindowRgn takes ownership of a successful region assignment.
fn update_monitor_region(mon: &MonitorRt, margin: f32) {
    let width = mon.compositor.width.max(0);
    let height = mon.compositor.height.max(0);
    let margin = margin.max(0.0);
    unsafe {
        let mut region = None;
        for block in mon.blocks.iter() {
            let left = (block.x - margin).floor().max(0.0) as i32;
            let top = (block.y - margin).floor().max(0.0) as i32;
            let right = (block.x + block.width + margin).ceil().min(width as f32) as i32;
            let bottom = (block.y + block.height + margin).ceil().min(height as f32) as i32;
            if left >= right || top >= bottom {
                continue;
            }
            let rect = CreateRectRgn(left, top, right, bottom);
            if rect.0.is_null() {
                continue;
            }
            if let Some(destination) = region {
                if CombineRgn(destination, destination, rect, RGN_OR) == GDI_REGION_TYPE(0) {
                    let _ = DeleteObject(rect);
                    let _ = DeleteObject(destination);
                    region = None;
                    break;
                }
                let _ = DeleteObject(rect);
            } else {
                region = Some(rect);
            }
        }

        let Some(region) = region else {
            // An empty region makes the overlay contribute no hit-testable area.
            // Passing a null HRGN would restore the full rectangle and block the
            // desktop again because WS_EX_TRANSPARENT is not a hit-test contract.
            let empty = CreateRectRgn(0, 0, 0, 0);
            if !empty.0.is_null() && SetWindowRgn(mon.hwnd, empty, true) == 0 {
                let _ = DeleteObject(empty);
            }
            return;
        };

        if SetWindowRgn(mon.hwnd, region, true) == 0 {
            let _ = DeleteObject(region);
        }
    }
}

/// 拖拽期间把窗口区域一次性扩到整窗。
/// 块外内容始终是全透明，扩大区域不改变任何像素；命中测试仍由 WM_NCHITTEST 的
/// HTTRANSPARENT 负责穿透。换来的是拖拽全程 0 次 SetWindowRgn —— 旧实现每帧
/// 重建 HRGN + SetWindowRgn，会让 DWM 反复重裁剪整个 overlay，是 100~280 ms
/// 主循环长尾的主要来源。
fn set_monitor_region_full(mon: &MonitorRt) {
    unsafe {
        let width = mon.compositor.width.max(1);
        let height = mon.compositor.height.max(1);
        let full = CreateRectRgn(0, 0, width, height);
        if full.0.is_null() {
            return;
        }
        if SetWindowRgn(mon.hwnd, full, false) == 0 {
            let _ = DeleteObject(full);
        }
    }
}

/// 等待各屏采集首帧就绪。主屏给足 primary_budget（必要时重建 duplication），
/// 其余屏各给 1.5 秒；拿不到的屏不显示画布，运行期仍会继续退避重试。
fn wait_for_captures(app: &mut App, primary_budget: std::time::Duration) {
    for mi in 0..app.monitors.len() {
        let budget = if mi == 0 {
            primary_budget
        } else {
            std::time::Duration::from_millis(1500)
        };
        let deadline = Instant::now() + budget;
        while !app.monitors[mi].compositor.capture_ready() && Instant::now() < deadline {
            let gfx = &app.gfx;
            let mon = &mut app.monitors[mi];
            // duplication 建不起来时按 250 ms 退避重试（方案 4.3）
            if mon.capture.is_none() {
                if Instant::now() < mon.retry_at {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    continue;
                }
                match DdaCapture::new_for_monitor(&gfx.device, mon.info.hmonitor) {
                    Ok(cap) => {
                        if mon.capture_error_logged {
                            eprintln!("[info] {} 采集已恢复", mon.info.name);
                        }
                        mon.capture_error_logged = false;
                        mon.capture = Some(cap);
                    }
                    Err(e) => {
                        // 只在状态翻转时打一次：锁屏期间会每 250 ms 重试一次。
                        if !mon.capture_error_logged {
                            mon.capture_error_logged = true;
                            eprintln!("[warn] {} 采集重试失败: {e}", mon.info.name);
                        }
                        mon.retry_at =
                            Instant::now() + std::time::Duration::from_millis(250);
                        continue;
                    }
                }
            }
            let outcome = match mon.capture.as_mut() {
                Some(cap) => cap.try_capture(&gfx.device, &gfx.ctx, 32),
                None => continue,
            };
            match outcome {
                Ok(Some((tex, w, h, changed))) => {
                    if mon.compositor.width != w || mon.compositor.height != h {
                        let _ = mon.compositor.resize(gfx, w, h);
                    }
                    let _ = mon.compositor.set_capture(gfx, tex, changed);
                }
                Ok(None) => {}
                Err(e) => {
                    eprintln!("[warn] {} DDA 首帧: {e}", mon.info.name);
                    mon.capture = None;
                    mon.retry_at =
                        Instant::now() + std::time::Duration::from_millis(250);
                }
            }
        }
        if !app.monitors[mi].compositor.capture_ready() {
            eprintln!(
                "[warn] {} 采集首帧超时，该屏暂不显示玻璃（运行期会继续重试）",
                app.monitors[mi].info.name
            );
        }
    }
}

/// 显示器拓扑变化后重建全部画布（方案 4.1：分辨率 / DPI / 拔插显示器）。
fn rebuild_monitors(app: &mut App) {
    let infos = monitors::enumerate();
    if infos.is_empty() {
        return;
    }
    // 先建新窗口再销毁旧窗口，避免中间态出现「一块屏都没有」。
    let old: Vec<HWND> = app.monitors.iter().map(|m| m.hwnd).collect();
    let mut next: Vec<MonitorRt> = Vec::new();
    for info in infos {
        if let Some(mut m) = MonitorRt::create(&app.gfx, app.class_name, app.hinst, info) {
            if let Err(e) = m.compositor.resize(&app.gfx, m.info.size[0], m.info.size[1]) {
                eprintln!("[warn] {} 金字塔初始化失败: {e}", m.info.name);
            }
            unsafe {
                SetWindowLongPtrW(m.hwnd, GWLP_USERDATA, app as *mut App as isize);
            }
            next.push(m);
        }
    }
    if next.is_empty() {
        return;
    }
    app.monitors = next;
    for hwnd in old {
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(hwnd);
        }
    }
    // 块重新归位到新的显示器集合
    let ids: Vec<u32> = app.manager.blocks().iter().map(|b| b.id).collect();
    for id in ids {
        if let Some(b) = app.manager.get_mut(id) {
            b.monitor = 0;
            b.scale = 1.0;
        }
        sync_block_monitor(app, id);
    }
    for m in app.monitors.iter_mut() {
        m.region_dirty = true;
    }
}

/// 渲染一块屏。返回是否真的提交了内容（脏区为空时为 false）。
fn render_monitor(app: &mut App, mi: usize, ctx: &FrameCtx, full_redraw: bool) -> bool {
    let App {
        gfx,
        glass,
        monitors,
        ..
    } = app;
    let mon = &mut monitors[mi];
    let origin = mon.canvas_origin();
    let prev_cursor_canvas = [
        ctx.prev_cursor_screen[0] - origin[0],
        ctx.prev_cursor_screen[1] - origin[1],
    ];
    let blocks: &[Block] = &mon.blocks;
    let order: &[usize] = &mon.order;
    let params: &Params = &mon.params;
    let dirty = mon.backdrop_dirty;
    let result = mon.compositor.render(
        gfx,
        glass,
        blocks,
        order,
        params,
        ctx.cursor_screen,
        origin,
        ctx.drag_velocity,
        ctx.time,
        ctx.press,
        ctx.hover_id,
        prev_cursor_canvas,
        full_redraw,
        dirty,
    );
    if let Err(e) = result {
        eprintln!("render({}): {e}", mon.info.name);
        return false;
    }
    mon.compositor.last_dirty_px > 0
}

/// 帧预算统计（方案 7.6）。全部按 5 秒墙钟窗口结算，与「这一窗口渲染了多少帧」
/// 无关 —— 否则静止时窗口会被无限拉长，让 avg/max 失去可比性。
struct Metrics {
    epoch: Instant,
    loops: u64,
    frames: u64,
    idle_wakes: u64,
    max_loop_us: u128,
    render_sum_us: u128,
    render_max_us: u128,
    dirty_sum: i64,
    dirty_max: i64,
    blocks_sum: i64,
    pass_a: u64,
    pass_a_px: i64,
    copied_px: i64,
    acquired: u64,
    content: u64,
}

impl Metrics {
    fn new() -> Self {
        Self {
            epoch: Instant::now(),
            loops: 0,
            frames: 0,
            idle_wakes: 0,
            max_loop_us: 0,
            render_sum_us: 0,
            render_max_us: 0,
            dirty_sum: 0,
            dirty_max: 0,
            blocks_sum: 0,
            pass_a: 0,
            pass_a_px: 0,
            copied_px: 0,
            acquired: 0,
            content: 0,
        }
    }
}

fn report_metrics(
    app: &mut App,
    m: &mut Metrics,
    loop_us: u128,
    render_us: u128,
    rendered: bool,
) {
    m.loops += 1;
    m.max_loop_us = m.max_loop_us.max(loop_us);
    if rendered {
        m.frames += 1;
        m.render_sum_us += render_us;
        m.render_max_us = m.render_max_us.max(render_us);
    } else {
        m.idle_wakes += 1;
    }
    for mon in app.monitors.iter_mut() {
        if rendered {
            m.dirty_sum += mon.compositor.last_dirty_px;
            m.dirty_max = m.dirty_max.max(mon.compositor.last_dirty_px);
            m.blocks_sum += mon.compositor.last_drew_blocks as i64;
            if mon.compositor.last_pass_a {
                m.pass_a += 1;
            }
            m.pass_a_px += mon.compositor.last_pass_a_px;
        }
        m.copied_px += mon.copied_px;
        mon.copied_px = 0;
        if let Some(cap) = mon.capture.as_mut() {
            m.acquired += cap.acquired;
            m.content += cap.content_changed;
            cap.acquired = 0;
            cap.content_changed = 0;
            cap.pointer_only = 0;
        }
    }

    let secs = m.epoch.elapsed().as_secs_f64();
    if secs < 5.0 {
        return;
    }
    let n = m.frames.max(1) as f64;
    let screens = app.monitors.len().max(1) as f64;
    let stage = app.monitors[0].compositor.last_stage_us;
    let (gpu_avg, gpu_max, gpu_n) = match app.monitors[0].compositor.gpu.as_mut() {
        Some(gpu) => gpu.take_stats(),
        None => ([0.0; 3], [0.0; 3], 0),
    };
    eprintln!(
        "[metrics] win_s={:.2} screens={} blocks={} loops={} frames={} fps={:.1} idle={} \
         avg_render_ms={:.3} max_render_ms={:.3} max_loop_ms={:.3} \
         stage_us=A{} B{} P{} C{} dirty_mpx_avg={:.3} dirty_mpx_max={:.3} \
         blocks_avg={:.1} pass_a={:.2}/frame pass_a_mpx_avg={:.3} \
         dda_acq={:.1}/s dda_changed={:.1}/s copy_mpx_avg={:.3} \
         gpu_ms_avg=A{:.3}/B{:.3}/T{:.3} gpu_ms_max=A{:.3}/B{:.3}/T{:.3} gpu_n={}",
        secs,
        app.monitors.len(),
        app.manager.len(),
        m.loops,
        m.frames,
        m.frames as f64 / secs,
        m.idle_wakes,
        m.render_sum_us as f64 / n / 1000.0,
        m.render_max_us as f64 / 1000.0,
        m.max_loop_us as f64 / 1000.0,
        stage[0],
        stage[1],
        stage[2],
        stage[3],
        m.dirty_sum as f64 / n / screens / 1e6,
        m.dirty_max as f64 / 1e6,
        m.blocks_sum as f64 / n / screens,
        m.pass_a as f64 / n,
        m.pass_a_px as f64 / n / screens / 1e6,
        m.acquired as f64 / secs,
        m.content as f64 / secs,
        m.copied_px as f64 / n / screens / 1e6,
        gpu_avg[0],
        gpu_avg[1],
        gpu_avg[2],
        gpu_max[0],
        gpu_max[1],
        gpu_max[2],
        gpu_n,
    );
    *m = Metrics::new();
}

fn main() {
    unsafe {
        if let Err(e) = RoInitialize(RO_INIT_MULTITHREADED) {
            eprintln!("WinRT 初始化失败: {e}");
            return;
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let infos = monitors::enumerate();
    if infos.is_empty() {
        eprintln!("未枚举到任何显示器");
        return;
    }
    eprintln!(
        "[info] 显示器: {}",
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

    let hmod = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
    let hinst = HINSTANCE(hmod.0);
    let class_name = windows::core::w!("LiquidGlassCanvas");
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_CLASSDC,
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinst,
        lpszClassName: class_name,
        ..Default::default()
    };
    if unsafe { RegisterClassExW(&wc) } == 0 {
        eprintln!("RegisterClassExW 失败");
        return;
    }

    let gfx = match Device::create() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("D3D11 初始化失败: {e}");
            return;
        }
    };
    if gfx.warp {
        eprintln!("[warn] 回退到 WARP 软件光栅");
    }
    // 着色器 / LUT / 采样器全进程一份，所有显示器画布共享。
    let glass = match GlassRenderer::new(&gfx.device) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("着色器初始化失败: {e}");
            return;
        }
    };

    let mut monitors_rt: Vec<MonitorRt> = Vec::new();
    for info in infos {
        if let Some(mut m) = MonitorRt::create(&gfx, class_name, hinst, info) {
            if let Err(e) = m.compositor.resize(&gfx, m.info.size[0], m.info.size[1]) {
                eprintln!("[warn] {} 金字塔初始化失败: {e}", m.info.name);
            }
            // 取证图按屏分开，便于逐屏核对多屏渲染结果。
            if let Ok(base) = std::env::var("GLASS_INTERNAL_SHOT") {
                m.compositor.shot_path =
                    Some(format!("{base}.m{}.bmp", monitors_rt.len()));
            }
            monitors_rt.push(m);
        }
    }
    if monitors_rt.is_empty() {
        eprintln!("没有任何显示器能创建画布");
        return;
    }

    // 默认先展示一块完整玻璃；需要压力测试时设置 GLASS_MANY=1。
    let mut manager = Manager::default();
    let (lw, lh) = (
        monitors_rt[0].info.size[0] as f32,
        monitors_rt[0].info.size[1] as f32,
    );
    if std::env::var("GLASS_MANY").ok().as_deref() == Some("1") {
        for i in 0..20 {
            let w = 600.0 + (i % 4) as f32 * 30.0;
            let h = 400.0 + (i % 3) as f32 * 24.0;
            manager.add_auto(i, w, h, lw, lh);
        }
    } else {
        let w = (lw * 0.62).clamp(720.0, 1200.0);
        let h = (lh * 0.68).clamp(480.0, 760.0);
        manager.add(((lw - w) * 0.5).max(8.0), ((lh - h) * 0.5).max(8.0), w, h);
    }
    // GLASS_SPREAD=1：在每块显示器正中各放一块玻璃，用于多屏验收。
    // 尺寸以「主屏物理像素」为逻辑单位，落到别的屏时按该屏 DPI 换算，
    // 因此在分辨率 / 缩放各不相同的屏上物理观感一致（方案 12.2.5）。
    if std::env::var("GLASS_SPREAD").ok().as_deref() == Some("1") {
        manager.clear();
        for mi in 0..monitors_rt.len() {
            let info = &monitors_rt[mi].info;
            let sc = info.scale.max(0.05);
            let w = 600.0 * sc;
            let h = 400.0 * sc;
            let x = info.origin[0] as f32 + (info.size[0] as f32 - w) * 0.5;
            let y = info.origin[1] as f32 + (info.size[1] as f32 - h) * 0.5;
            let id = manager.add(x, y, w, h);
            if let Some(b) = manager.get_mut(id) {
                b.monitor = mi;
                b.scale = sc;
                b.radius = 32.0 * sc;
            }
        }
    }

    // 中文控制面板（内嵌 HTTP 服务 + 系统浏览器）
    let shared: &'static ui::SharedParams = Box::leak(Box::new(ui::SharedParams::default()));
    if let Err(e) = ui::panel::create(shared) {
        eprintln!("[warn] 控制面板创建失败: {e}");
    }
    shared
        .new_width
        .store(600, std::sync::atomic::Ordering::Relaxed);
    shared
        .new_height
        .store(400, std::sync::atomic::Ordering::Relaxed);

    let cursor_start = [
        monitors_rt[0].info.origin[0] as f32 + monitors_rt[0].info.size[0] as f32 * 0.5,
        monitors_rt[0].info.origin[1] as f32 + monitors_rt[0].info.size[1] as f32 * 0.5,
    ];
    let mut app = App {
        gfx,
        glass,
        monitors: monitors_rt,
        manager,
        params: {
            let mut p = Params::default();
            // 极端回归模式：用最大高光强度、最小高光锐度验证
            // bevel/plane 和圆角转角不会出现断裂。视觉效果可通过面板恢复。
            if std::env::var("GLASS_EXTREME_SPEC").ok().as_deref() == Some("1") {
                p.spec_intensity = 4.0;
                p.spec_power = 1.0;
            }
            p
        },
        shared,
        hinst,
        class_name,
        last_block_action: 0,
        drag: None,
        hover: None,
        cursor: cursor_start,
        press: 0.0,
        drag_velocity: [0.0, 0.0],
        rebuild_monitors: false,
        t0: Instant::now(),
        frames: 0,
    };
    app.params.sanitize();
    let ids: Vec<u32> = app.manager.blocks().iter().map(|b| b.id).collect();
    for id in ids {
        sync_block_monitor(&mut app, id);
    }
    sync_selection(&app);
    let _ = app.glass.update_lut(&app.gfx.ctx, &app.params);

    for mi in 0..app.monitors.len() {
        unsafe {
            SetWindowLongPtrW(
                app.monitors[mi].hwnd,
                GWLP_USERDATA,
                &mut app as *mut App as isize,
            );
        }
        collect_monitor_blocks(&mut app, mi, true);
        let margin = app.params.shadow_margin.max(0.0);
        update_monitor_region(&app.monitors[mi], margin);
    }

    // overlay 显示前先等到各屏采集就绪，从根源消除初始黑帧。
    // DuplicateOutput 可能因「另一个进程占用 / 独占全屏 / 驱动尚未释放」而
    // 返回 E_ACCESSDENIED 或 NOT_CURRENTLY_AVAILABLE（方案 4.3），因此这里
    // 必须带退避重试；重试期间进程保持存活，成功后再显示对应屏的画布。
    wait_for_captures(&mut app, std::time::Duration::from_secs(6));
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
        collect_monitor_blocks(&mut app, mi, true);
        if app.monitors[mi].compositor.capture_ready() {
            let _ = render_monitor(&mut app, mi, &initial, true);
        }
        // 只有拿到真实采集帧的屏才允许显示。
        app.monitors[mi].ready = app.monitors[mi].compositor.capture_ready();
    }
    for mon in app.monitors.iter() {
        if mon.ready {
            mon.show();
            eprintln!("[info] {} 画布已显示 ({}x{})", mon.info.name, mon.info.size[0], mon.info.size[1]);
        }
    }

    // ---- 主循环：消息泵 + 事件驱动渲染（方案 10.4 / 12.4.1）----
    // 触发源为空时整帧跳过：不跑 Pass B、不 Present、不 Commit。
    let metrics_enabled = std::env::var("GLASS_METRICS").ok().as_deref() == Some("1");
    // 旧实现每帧 std::env::var 会加进程环境锁并分配 String，这里只读一次。
    let extreme_spec = std::env::var("GLASS_EXTREME_SPEC").ok().as_deref() == Some("1");
    // GLASS_VERIFY_PASS_A=1：拍一张「脏矩形受限 Pass A」的帧后，立刻用同一张
    // capture 强制跑一次「全量 Pass A + 全量 Pass B」，输出 .full.bmp 对照图。
    let verify_pass_a = std::env::var("GLASS_VERIFY_PASS_A").ok().as_deref() == Some("1");
    let verify_shot_path = std::env::var("GLASS_INTERNAL_SHOT").ok();
    let mut verify_done = false;
    // 静止时的等待步长：8 ms ≈ 125 Hz，覆盖 120 Hz 显示器的一个刷新周期，
    // 保证鼠标悬停反馈延迟不超过半帧，同时静止桌面 CPU 趋近 0。
    const IDLE_WAIT_MS: u32 = 8;
    let mut capture_wait_ms = IDLE_WAIT_MS;
    let mut metrics = Metrics::new();
    let mut msg = MSG::default();
    loop {
        let loop_start = Instant::now();
        while unsafe { PeekMessageW(&mut msg, HWND::default(), 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == WM_QUIT {
                return;
            }
            let _ = unsafe { TranslateMessage(&msg) };
            unsafe { DispatchMessageW(&msg) };
        }

        if app.rebuild_monitors {
            app.rebuild_monitors = false;
            rebuild_monitors(&mut app);
        }

        // ---- 每屏采集 ----
        // DDA 的阻塞等待同时充当主循环唯一的休眠点（方案 12.4.1「以采集帧为
        // 主时钟」）：桌面有新内容立即返回，静止时在驱动里内核等待。
        let mut new_backdrop = false;
        {
            let gfx = &app.gfx;
            for mon in app.monitors.iter_mut() {
                mon.new_backdrop = false;
                mon.backdrop_dirty = None;
                mon.copied_px = 0;
                let (outcome, dirty, copied, lost) = match mon.capture.as_mut() {
                    Some(cap) => {
                        let r = cap.try_capture(&gfx.device, &gfx.ctx, capture_wait_ms);
                        (r, cap.dirty_union, cap.copied_px, cap.access_lost)
                    }
                    // 没有采集源：到点再尝试重建（独占全屏 / 录屏软件抢占后恢复）
                    None => {
                        if Instant::now() >= mon.retry_at {
                            mon.retry_at =
                                Instant::now() + std::time::Duration::from_millis(500);
                            match DdaCapture::new_for_monitor(&gfx.device, mon.info.hmonitor) {
                                Ok(cap) => {
                                    if mon.capture_error_logged {
                                        eprintln!("[info] {} 采集已恢复", mon.info.name);
                                    }
                                    mon.capture_error_logged = false;
                                    mon.capture = Some(cap);
                                    // 采集源重建后金字塔与后备缓冲都失效，
                                    // 下一帧必须整屏重来。
                                    mon.compositor.invalidate();
                                }
                                Err(_) => {
                                    if !mon.capture_error_logged {
                                        mon.capture_error_logged = true;
                                        eprintln!(
                                            "[warn] {} 采集不可用，500 ms 后退避重试",
                                            mon.info.name
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
                                eprintln!("resize({}): {e}", mon.info.name);
                            }
                        }
                        if let Err(e) = mon.compositor.set_capture(gfx, tex, changed) {
                            eprintln!("set_capture({}): {e}", mon.info.name);
                        }
                        mon.new_backdrop = changed;
                        mon.backdrop_dirty = dirty;
                        mon.copied_px = copied;
                        if !mon.ready && mon.compositor.capture_ready() {
                            mon.ready = true;
                            mon.show();
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        // ACCESS_LOST / 锁屏（安全桌面）：释放并安排退避重建，
                        // 绝不在错误上忙等。锁屏时 DDA 会持续返回 E_ACCESSDENIED，
                        // 解锁后自动恢复，无需人工干预。
                        if !mon.capture_error_logged {
                            mon.capture_error_logged = true;
                            eprintln!(
                                "[warn] {} DDA 失效（锁屏/独占全屏/被其他采集占用？）: {e}；                                 每 500 ms 退避重试，恢复后自动继续",
                                mon.info.name
                            );
                        }
                        mon.retry_at =
                            Instant::now() + std::time::Duration::from_millis(500);
                        mon.capture = None;
                        if mon.ready {
                            // 画面会冻结在最后一帧，且仍压在独占全屏应用之上，
                            // 因此采集一失效就隐藏画布，恢复后再显示（方案 R-16）。
                            unsafe {
                                let _ = ShowWindow(mon.hwnd, SW_HIDE);
                            };
                        }
                        mon.ready = false;
                    }
                }
                let _ = lost;
                if mon.new_backdrop {
                    new_backdrop = true;
                }
            }
        }

        // ---- 控制面板参数同步（每帧读取原子值，无锁） ----
        let previous_params = app.params.clone();
        app.shared.apply(&mut app.params);
        if extreme_spec {
            // Keep the requested extreme regression values after panel sync.
            app.params.spec_intensity = 4.0;
            app.params.spec_power = 1.0;
        }
        let params_changed = app.params != previous_params;
        if params_changed && (app.params.ior - previous_params.ior).abs() > f32::EPSILON {
            if let Err(e) = app.glass.update_lut(&app.gfx.ctx, &app.params) {
                eprintln!("update LUT: {e}");
            }
            // 参数变化只影响 Pass B 的 shader 常量，不需要重算背景金字塔。
        }

        // 面板「宽度/高度」作用于当前选中块（逻辑单位，跨屏自动按 DPI 换算）；
        // 「圆角半径」滑杆作为全局默认。
        let radius_slider = app.params.corner_radius;
        let target_w = app
            .shared
            .new_width
            .load(std::sync::atomic::Ordering::Relaxed) as f32;
        let target_h = app
            .shared
            .new_height
            .load(std::sync::atomic::Ordering::Relaxed) as f32;
        let target_id = app
            .shared
            .selected_id
            .load(std::sync::atomic::Ordering::Relaxed);
        let dragging = app.drag.is_some();
        let mut geometry_changed = false;
        if let Some(id) = (target_id >= 0 && !dragging).then_some(target_id as u32) {
            if let Some(b) = app.manager.get_mut(id) {
                let s = b.scale.max(1e-3);
                if target_w >= 24.0 && (b.width - target_w * s).abs() > 0.5 {
                    b.width = target_w * s;
                    geometry_changed = true;
                }
                if target_h >= 24.0 && (b.height - target_h * s).abs() > 0.5 {
                    b.height = target_h * s;
                    geometry_changed = true;
                }
                b.radius = (radius_slider * s).min(0.5 * b.width.min(b.height));
                b.clamp();
            }
        } else {
            for b in app.manager.blocks_mut().iter_mut() {
                b.radius = (radius_slider * b.scale).min(0.5 * b.width.min(b.height));
            }
        }

        // 面板里的块操作（一次性消费）
        let action = app
            .shared
            .block_action
            .swap(0, std::sync::atomic::Ordering::AcqRel);
        if action != 0 {
            let nw = app
                .shared
                .new_width
                .load(std::sync::atomic::Ordering::Relaxed)
                .max(24) as f32;
            let nh = app
                .shared
                .new_height
                .load(std::sync::atomic::Ordering::Relaxed)
                .max(24) as f32;
            let [sw, sh] = app.primary_size();
            match action {
                1 => {
                    let i = app.manager.len();
                    let id = app.manager.add_auto(i, nw, nh, sw, sh);
                    sync_block_monitor(&mut app, id);
                    geometry_changed = true;
                    app.shared
                        .selected_id
                        .store(id as isize, std::sync::atomic::Ordering::Relaxed);
                }
                2 => {
                    let selected = app
                        .shared
                        .selected_id
                        .load(std::sync::atomic::Ordering::Relaxed);
                    let id = (selected >= 0)
                        .then_some(selected as u32)
                        .filter(|id| app.manager.get(*id).is_some())
                        .or_else(|| app.manager.blocks().last().map(|b| b.id));
                    if let Some(id) = id {
                        app.manager.remove(id);
                        geometry_changed = true;
                    }
                    sync_selection(&app);
                }
                3 => {
                    app.manager.clear();
                    geometry_changed = true;
                    app.shared
                        .selected_id
                        .store(-1, std::sync::atomic::Ordering::Relaxed);
                }
                4 => {
                    let mut last = -1isize;
                    for _ in 0..20 {
                        let i = app.manager.len();
                        let id = app.manager.add_auto(i, nw, nh, sw, sh);
                        sync_block_monitor(&mut app, id);
                        last = id as isize;
                    }
                    geometry_changed = true;
                    app.shared
                        .selected_id
                        .store(last, std::sync::atomic::Ordering::Relaxed);
                }
                _ => {}
            }
            app.last_block_action = action;
            sync_selection(&app);
        }

        // ---- 鼠标状态（每帧轮询，无 hook；方案 6.3） ----
        let previous_cursor = app.cursor;
        let p = cursor_pos();
        app.cursor = [p.x as f32, p.y as f32];
        let cursor_moved = (app.cursor[0] - previous_cursor[0]).abs() > 0.01
            || (app.cursor[1] - previous_cursor[1]).abs() > 0.01;
        let new_hover = match app.drag.as_ref() {
            Some(d) => Some(d.id),
            None => app.manager.hit_test(app.cursor[0], app.cursor[1]),
        };
        let hover_changed = new_hover != app.hover;
        app.hover = new_hover;

        // ---- 拖拽：每帧应用一次（方案 12.2.3 输入合并） ----
        // 先算好拖拽上下文，避免在 app.drag 的可变借用里再借用 app。
        let drag_id = app.drag.as_ref().map(|d| d.id);
        let drag_scale = drag_id.map_or(1.0, |id| {
            app.scale_of(app.manager.get(id).map_or(0, |b| b.monitor))
        });
        let drag_monitor = match app.drag.as_mut() {
            Some(drag) => {
                let now = app.t0.elapsed().as_secs_f64();
                let id = drag.id;
                let resizing = drag.kind == DragKind::ResizeSE;
                // 用块**当前**所在屏的比例换算；跨屏后 sync_block_monitor 会再校正。
                let scale = drag_scale;
                let r = drag.apply(app.cursor, now, scale);
                app.drag_velocity = drag.velocity;
                if let Some(b) = app.manager.get_mut(id) {
                    b.x = r[0];
                    b.y = r[1];
                    // 移动时绝不写回尺寸：尺寸归 DPI 换算管，否则跨屏后会被
                    // 按下那一刻的像素尺寸覆盖（跨屏回归测试抓到的真实缺陷）。
                    if resizing {
                        b.width = r[2] * scale;
                        b.height = r[3] * scale;
                    }
                    b.radius = b.radius.min(0.5 * b.width.min(b.height));
                    b.clamp();
                }
                // 跨屏：中心越过边界就换屏，并按两屏 DPI 比例换算尺寸。
                sync_block_monitor(&mut app, id);
                Some(
                    app.manager
                        .get(id)
                        .map_or(0, |b| b.monitor),
                )
            }
            None => {
                app.drag_velocity[0] *= 0.85;
                app.drag_velocity[1] *= 0.85;
                None
            }
        };
        if drag_monitor.is_some() {
            geometry_changed = true;
        }

        // ---- 每屏：投影块 + 窗口区域 + 点击穿透 ----
        let margin = app.params.shadow_margin.max(0.0);
        let influence = app.params.mouse_radius * 2.5 + 32.0 + margin;
        let mut mouse_relevant = false;
        for mi in 0..app.monitors.len() {
            collect_monitor_blocks(&mut app, mi, params_changed);
            let mon_dragging = drag_monitor == Some(mi);
            // 拖拽只影响被拖块所在的那块屏；其他屏的区域不必重建
            // （SetWindowRgn 会让 DWM 重裁剪整个 overlay，是长尾的主要来源）。
            let region_idle = drag_monitor.is_some() && !mon_dragging;
            if mon_dragging {
                if !app.monitors[mi].drag_region_full {
                    set_monitor_region_full(&app.monitors[mi]);
                    app.monitors[mi].drag_region_full = true;
                }
            } else if !region_idle
                && (app.monitors[mi].drag_region_full
                    || app.monitors[mi].region_dirty
                    || geometry_changed)
            {
                update_monitor_region(&app.monitors[mi], margin);
                app.monitors[mi].drag_region_full = false;
                app.monitors[mi].region_dirty = false;
            }

            // 光标是否可能影响本屏任一块（鼠标点光是屏幕空间场）
            let origin = app.monitors[mi].canvas_origin();
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
            if cursor_moved
                && mon.blocks.iter().any(|blk| {
                    blk.x < x1 && blk.x + blk.width > x0 && blk.y < y1 && blk.y + blk.height > y0
                })
            {
                mouse_relevant = true;
            }

            // 点击穿透：光标在本屏某块内 -> 取消穿透；否则恢复（方案 12.1.4）
            let inside = mon_dragging
                || mon.blocks.iter().any(|blk| {
                    b[0] >= blk.x
                        && b[0] <= blk.x + blk.width
                        && b[1] >= blk.y
                        && b[1] <= blk.y + blk.height
                });
            let hwnd = mon.hwnd;
            unsafe {
                let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as i64;
                let transparent = (style & WS_EX_TRANSPARENT.0 as i64) != 0;
                if inside && transparent {
                    SetWindowLongPtrW(
                        hwnd,
                        GWL_EXSTYLE,
                        (style & !(WS_EX_TRANSPARENT.0 as i64)) as isize,
                    );
                } else if !inside && !transparent {
                    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (style | WS_EX_TRANSPARENT.0 as i64) as isize);
                }
            }
        }

        // ---- 渲染门控（方案 12.4.1）：没有任何触发源就整帧跳过 ----
        let velocity_active =
            app.drag_velocity[0].abs() >= 0.01 || app.drag_velocity[1].abs() >= 0.01;
        let requested = RENDER_REQUEST.swap(false, std::sync::atomic::Ordering::Acquire);
        let need_render = new_backdrop
            || params_changed
            || geometry_changed
            || hover_changed
            || mouse_relevant
            || dragging
            || velocity_active
            || requested;

        if !need_render {
            // 静止：不重画、不 Present、不 Commit。
            capture_wait_ms = IDLE_WAIT_MS;
            // 速率下限兜底：正常情况下 AcquireNextFrame 已经在驱动里阻塞了
            // IDLE_WAIT_MS；一旦采集源失效并立即返回，这里保证主循环绝不会
            // 退化成数十 kHz 的空转（实测缺陷：DDA ACCESS_LOST 时 96 kHz / 83% CPU）。
            let elapsed = loop_start.elapsed();
            let floor = std::time::Duration::from_millis(IDLE_WAIT_MS as u64);
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
        let mut presented = false;
        for mi in 0..app.monitors.len() {
            if !app.monitors[mi].ready {
                continue;
            }
            if render_monitor(&mut app, mi, &ctx, false) {
                presented = true;
            }
        }
        let render_us = render_start.elapsed().as_micros();

        // 自证：对每一块屏，用同一张 capture 立刻再渲染一帧「全量 Pass A +
        // 全量 Pass B（整屏清屏 + 全部块重画）」，输出 .full.bmp 对照图。
        // 两张 overlay 必须逐像素一致，否则说明脏区 / 采样余量算漏了。
        if verify_pass_a && !verify_done {
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
                        eprintln!("[verify] 已输出全量对照帧: {base}.m{mi}.full.bmp");
                    }
                }
            }
        }

        if presented {
            app.frames += 1;
        }
        if metrics_enabled {
            report_metrics(
                &mut app,
                &mut metrics,
                loop_start.elapsed().as_micros(),
                render_us,
                presented,
            );
        }
    }
}
