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
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, GetMonitorInfoW, MonitorFromWindow, SetWindowRgn,
    UpdateWindow, GDI_REGION_TYPE, MONITORINFO, MONITOR_DEFAULTTONEAREST, RGN_OR,
};
use windows::Win32::Graphics::Gdi::{EnumDisplaySettingsW, DEVMODEW, ENUM_CURRENT_SETTINGS};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::*;

use capture::Capture;
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
    capture: Option<Capture>,
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
    /// 上次采集时刻：没有玻璃块的屏降到 4 Hz 采集，省掉整条采集开销。
    last_capture: Instant,
    /// 本屏当前处于「无块降频采集」状态，用于识别块刚落入本屏的那一帧。
    idle_capturing: bool,
    /// 因「独占全屏应用」被我们主动隐藏（方案 R-16）
    hidden_fs: bool,
    /// 因「采集失效」被我们主动隐藏
    hidden_capture: bool,
    /// 因「本屏没有任何玻璃块」被隐藏。
    /// 实测：在 60 Hz 副屏上挂一个 DComp 画布（哪怕整帧短路、一次 Present 都不发），
    /// 也会把 120 Hz 主屏的合成节拍拖到 60 Hz —— 20 块拖拽 fps 82.0 -> 42.3、
    /// p50 单循环 8.41 ms -> 16.67 ms。所以空屏必须把画布真正隐藏掉。
    hidden_empty: bool,
    /// 本屏变空的时刻（1 秒防抖，避免跨屏拖拽时反复显示/隐藏）
    empty_since: Option<Instant>,
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
                WS_EX_NOREDIRECTIONBITMAP
                    | WS_EX_TOPMOST
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE,
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
        let capture = match Capture::for_monitor(&gfx.device, info.hmonitor) {
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
            last_capture: Instant::now().checked_sub(std::time::Duration::from_secs(3600)).unwrap_or_else(std::time::Instant::now),
            idle_capturing: false,
            hidden_fs: false,
            hidden_capture: false,
            hidden_empty: false,
            empty_since: None,
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
    fs_check_at: Instant,
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
        let mut local = *b;
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
                match Capture::for_monitor(&gfx.device, mon.info.hmonitor) {
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

/// 画布可见性 = 采集就绪 且 未被独占全屏应用压制（方案 R-16）。
fn apply_visibility(mon: &MonitorRt) {
    unsafe {
        if mon.ready && !mon.hidden_fs && !mon.hidden_capture && !mon.hidden_empty {
            mon.show();
        } else {
            let _ = ShowWindow(mon.hwnd, SW_HIDE);
        }
    }
}

/// 独占全屏应用检测（方案 R-16 / 12.1.1 C 模式的已知代价）。
///
/// 全屏置顶画布会破坏独占全屏与 MPO。判据刻意收紧为「无边框 popup 且恰好铺满
/// 所在显示器」——普通最大化窗口带 WS_CAPTION，不会被误判，玻璃照常叠加在上面。
/// 该窗口是不是我们自己的（画布类名固定），用来避免把自家 overlay 误判成
/// 独占全屏应用 —— 画布本身就是 WS_POPUP + 铺满显示器，完全符合"全屏"的形状判据。
fn is_own_window(hwnd: HWND) -> bool {
    if hwnd.0.is_null() {
        return false;
    }
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n]) == "LiquidGlassCanvas"
}

fn fullscreen_app_on(mon_hwnd: HWND) -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() || fg == mon_hwnd || is_own_window(fg) {
            return false;
        }
        let style = GetWindowLongPtrW(fg, GWL_STYLE) as u32;
        // 无边框全屏：有 WS_POPUP，没有标题栏、没有可拖拽边框
        if style & WS_POPUP.0 == 0 {
            return false;
        }
        if style & (WS_CAPTION.0 | WS_THICKFRAME.0) != 0 {
            return false;
        }
        let mut wr = RECT::default();
        if GetWindowRect(fg, &mut wr).is_err() {
            return false;
        }
        let hmon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(hmon, &mut mi).as_bool() {
            return false;
        }
        let m = mi.rcMonitor;
        wr.left <= m.left && wr.top <= m.top && wr.right >= m.right && wr.bottom >= m.bottom
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
    pass_a_throttled: u64,
    copied_px: i64,
    acquired: u64,
    content: u64,
    /// 主循环分段 CPU 耗时累加：消息泵 / 采集 / 渲染前准备
    msg_sum_us: u128,
    cap_sum_us: u128,
    cap_max_us: u128,
    prep_sum_us: u128,
    /// cap 段细分：AcquireNextFrame / 脏矩形 API / 像素搬运 / 取帧前置 / ReleaseFrame
    acq_sum_us: u128,
    acq_max_us: u128,
    dirtyapi_sum_us: u128,
    copy_sum_us: u128,
    pre_sum_us: u128,
    rel_sum_us: u128,
    rel_max_us: u128,
    /// 逐次循环采样 [总耗时, 采集段, 渲染段]。
    /// 每个窗口只有几十次循环时，一次几百毫秒的离群值就能把均值彻底带偏
    /// （实测出现过"跳过工作反而更慢"的因果倒置），所以改报分位数。
    samples: Vec<[u128; 3]>,
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
            pass_a_throttled: 0,
            copied_px: 0,
            acquired: 0,
            content: 0,
            msg_sum_us: 0,
            cap_sum_us: 0,
            cap_max_us: 0,
            prep_sum_us: 0,
            acq_sum_us: 0,
            acq_max_us: 0,
            dirtyapi_sum_us: 0,
            copy_sum_us: 0,
            pre_sum_us: 0,
            rel_sum_us: 0,
            rel_max_us: 0,
            samples: Vec::new(),
        }
    }
}

/// `stage_us` = [消息泵, 采集, 渲染前准备]
fn report_metrics(
    app: &mut App,
    m: &mut Metrics,
    loop_us: u128,
    render_us: u128,
    rendered: bool,
    stage_us: [u128; 3],
) {
    m.loops += 1;
    m.max_loop_us = m.max_loop_us.max(loop_us);
    m.msg_sum_us += stage_us[0];
    m.cap_sum_us += stage_us[1];
    m.cap_max_us = m.cap_max_us.max(stage_us[1]);
    m.prep_sum_us += stage_us[2];
    if m.samples.len() < 20000 {
        m.samples.push([loop_us, stage_us[1], render_us]);
    }
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
            m.pass_a_throttled += mon.compositor.pass_a_throttled;
            mon.compositor.pass_a_throttled = 0;
            m.pass_a_px += mon.compositor.last_pass_a_px;
        }
        m.copied_px += mon.copied_px;
        mon.copied_px = 0;
        if let Some(cap) = mon.capture.as_mut() {
            let (a, c) = cap.take_stats();
            m.acquired += a;
            m.content += c;
            let t = cap.take_timings();
            m.acq_sum_us += t[0];
            m.dirtyapi_sum_us += t[1];
            m.copy_sum_us += t[2];
            m.pre_sum_us += t[3];
            m.rel_sum_us += t[4];
            m.acq_max_us = m.acq_max_us.max(t[5]);
            m.rel_max_us = m.rel_max_us.max(t[6]);
        }
    }

    let secs = m.epoch.elapsed().as_secs_f64();
    if secs < 5.0 {
        return;
    }
    // 分位数：对离群值稳健
    fn pct(v: &mut Vec<u128>, p: f64) -> f64 {
        if v.is_empty() {
            return 0.0;
        }
        v.sort_unstable();
        let i = ((v.len() as f64 - 1.0) * p).round() as usize;
        v[i.min(v.len() - 1)] as f64 / 1000.0
    }
    let mut s_loop: Vec<u128> = m.samples.iter().map(|x| x[0]).collect();
    let mut s_cap: Vec<u128> = m.samples.iter().map(|x| x[1]).collect();
    let mut s_ren: Vec<u128> = m.samples.iter().map(|x| x[2]).filter(|x| *x > 0).collect();
    let loop_p50 = pct(&mut s_loop, 0.50);
    let loop_p95 = pct(&mut s_loop, 0.95);
    let cap_p50 = pct(&mut s_cap, 0.50);
    let cap_p95 = pct(&mut s_cap, 0.95);
    let ren_p50 = pct(&mut s_ren, 0.50);
    let ren_p95 = pct(&mut s_ren, 0.95);
    let n = m.frames.max(1) as f64;
    let nl = m.loops.max(1) as f64;
    let screens = app.monitors.len().max(1) as f64;
    let stage = app.monitors[0].compositor.last_stage_us;
    let (gpu_avg, gpu_max, gpu_n) = match app.monitors[0].compositor.gpu.as_mut() {
        Some(gpu) => gpu.take_stats(),
        None => ([0.0; 3], [0.0; 3], 0),
    };
    eprintln!(
        "[metrics] win_s={:.2} screens={} blocks={} loops={} frames={} fps={:.1} idle={} \
         avg_render_ms={:.3} max_render_ms={:.3} max_loop_ms={:.3} \
         stage_us=A{} B{} P{} C{} \
         loop_ms_avg=msg{:.3}/cap{:.3}/prep{:.3} cap_max_ms={:.3} \
         p50_ms=loop{:.2}/cap{:.2}/ren{:.2} p95_ms=loop{:.2}/cap{:.2}/ren{:.2} \
         cap_split_ms=acq{:.3}/dirtyApi{:.3}/copy{:.3}/pre{:.3}/rel{:.3} \
         acq_max_ms={:.3} rel_max_ms={:.3} \
         dirty_mpx_avg={:.3} dirty_mpx_max={:.3} \
         blocks_avg={:.1} pass_a={:.2}/frame pass_a_throttled={:.2}/frame \
         pass_a_mpx_avg={:.3} \
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
        m.msg_sum_us as f64 / nl / 1000.0,
        m.cap_sum_us as f64 / nl / 1000.0,
        m.prep_sum_us as f64 / nl / 1000.0,
        m.cap_max_us as f64 / 1000.0,
        loop_p50,
        cap_p50,
        ren_p50,
        loop_p95,
        cap_p95,
        ren_p95,
        m.acq_sum_us as f64 / nl / 1000.0,
        m.dirtyapi_sum_us as f64 / nl / 1000.0,
        m.copy_sum_us as f64 / nl / 1000.0,
        m.pre_sum_us as f64 / nl / 1000.0,
        m.rel_sum_us as f64 / nl / 1000.0,
        m.acq_max_us as f64 / 1000.0,
        m.rel_max_us as f64 / 1000.0,
        m.dirty_sum as f64 / n / screens / 1e6,
        m.dirty_max as f64 / 1e6,
        m.blocks_sum as f64 / n / screens,
        m.pass_a as f64 / n,
        m.pass_a_throttled as f64 / n,
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

/// 显示器刷新率（方案 12.4.2 帧节奏 / 12.4.3 质量预算的基准）
fn refresh_hz() -> f32 {
    unsafe {
        let mut dev = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        if EnumDisplaySettingsW(None, ENUM_CURRENT_SETTINGS, &mut dev).as_bool()
            && dev.dmDisplayFrequency > 1
        {
            return dev.dmDisplayFrequency as f32;
        }
    }
    60.0
}

fn on_battery() -> bool {
    unsafe {
        let mut st = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut st).is_ok() {
            // BatteryFlag 128 = 无电池（台式机）；ACLineStatus 0 = 未接电源
            return st.BatteryFlag != 128 && st.ACLineStatus == 0;
        }
    }
    false
}

/// 自适应质量控制器（方案 12.4.3）。
///
/// `level 0` = 最高画质 = 本项目的默认视觉，**与未启用控制器时逐位一致**。
/// 只有实测持续丢 vblank 时才逐级降级，恢复后逐级升回：
///
/// 档位顺序按**实测收益**排，而不是按方案原文排：方案把"关焦散/关色散"放在
/// 限帧之前，但实测这两档只省少量 ALU、几乎不换帧率，会让控制器在真正有效的
/// 手段之前就白白牺牲视觉。实测收益：frost 半分辨率 +14% fps / −36.5 MB，
/// 限帧 60 直接把抖动变成平滑 60 fps，关焦散/关色散 ≈ 0。
///
/// | level | 动作 | 实测收益 |
/// |---|---|---|
/// | 1 | `frost` 保持 1/2 分辨率（省掉 Pass A 的整屏 tent 上采样） | +14% fps、−36.5 MB；模糊层差异 ≤3/255 |
/// | 2 | + 帧率上限 60（把"忽 8.33 忽 16.67 ms"的抖动变成恒定 16.67 ms） | 消除 judder |
/// | 3 | + 关闭焦散与 Beer-Lambert 吸收 | 少量 ALU（电池/过热时的余量） |
/// | 4 | + 关闭 RGB 色散（3 tap → 1 tap） | 省 2 次采样/像素 |
///
/// 判据用「丢 vblank 的比例」而不是 GPU timestamp：timestamp query 会打断驱动的
/// 命令批处理（实测把帧率打到 15 fps），用它做闭环会自我实现地降级。
/// 丢帧比例 = **我们自己那一帧的工作量**（渲染前准备 + Pass A + Pass B + Commit）
/// > 1.25 × 垂直消隐间隔 的**渲染帧**占比。
///
/// 为什么必须排除两段阻塞等待（它们都不是渲染成本）：
/// 1. 采集阶段等桌面新帧：桌面安静时驱动侧一等就是 8~32 ms；
/// 2. Present 等 vblank / DWM 背压：实测 6.6~25 ms（1~3 个消隐）。
///
/// 旧判据用整循环时长，把这两段都算成「丢帧」，结果单块拖拽也被判成
/// 「1 秒窗口 56 帧中丢 vblank 55%」、一路降到 level 2 —— 而我们自己的工作只有
/// 50 µs/帧（8.33 ms 预算的 1/160）。代价是白白把 frost 降成半分辨率（视觉改动），
/// fps 一点没变好（51.8 -> 51.0，瓶颈在触发节拍与 DWM，不在渲染）。
///
/// 已知取舍（诚实记录）：这个判据对「纯 GPU/DWM 背压」是盲的 —— 那种情况下
/// Present 阻塞变长而我们的提交时间不变。选它是因为误降档在本机是**每次拖拽
/// 必现**的实测缺陷，而纯 GPU 背压从未实测到过（最坏窗口里 A+B+C 仍只有 50 µs）。
/// 若将来真出现 GPU 背压场景，正确做法是先用 `DwmGetCompositionTimingInfo`
/// 拿到 DWM 实际合成节拍（混合刷新率下会变成 60 Hz），再按那个节拍判丢帧。
///
/// 防抖：连续 2 个 1 秒窗口超阈值才降档；连续 4 个窗口干净**且距上次降档 ≥20 s**
/// 才升档。后一条是为了避免周期振荡 —— 降到 1 档后变流畅、立刻升回 0 档又卡、
/// 再降，如此往复会让用户每隔二十几秒看到一次抖动。持续重负载下控制器会稳定
/// 停在 1 档，只有负载真的掉下来（删块、桌面变静）才升回。
///
/// `GLASS_ADAPTIVE=0` 可整体关闭（回到"永远 level 0"）。
struct QualityCtrl {
    enabled: bool,
    level: u8,
    vblank_us: u128,
    frames: u64,
    missed: u64,
    epoch: Instant,
    cooldown_until: Instant,
    last_downgrade: Instant,
    good_streak: u8,
    bad_streak: u8,
    min_interval: std::time::Duration,
    last_present: Instant,
    /// 升档探针的最小间隔。升上去 10 秒内又被打回 = 失败探针，间隔翻倍
    /// （上限 300 s），避免在"撑不住的档位"和"撑得住的档位"之间周期振荡。
    upgrade_gate: std::time::Duration,
    last_upgrade: Instant,
}

impl QualityCtrl {
    fn new(hz: f32) -> Self {
        Self {
            enabled: std::env::var("GLASS_ADAPTIVE").ok().as_deref() != Some("0"),
            level: 0,
            vblank_us: (1_000_000.0 / hz.max(1.0)) as u128,
            frames: 0,
            missed: 0,
            epoch: Instant::now(),
            cooldown_until: Instant::now(),
            last_downgrade: Instant::now().checked_sub(std::time::Duration::from_secs(3600)).unwrap_or_else(std::time::Instant::now),
            good_streak: 0,
            bad_streak: 0,
            min_interval: std::time::Duration::from_secs(0),
            last_present: Instant::now(),
            upgrade_gate: std::time::Duration::from_secs(20),
            last_upgrade: Instant::now().checked_sub(std::time::Duration::from_secs(3600)).unwrap_or_else(std::time::Instant::now),
        }
    }

    /// level 4 的帧率上限：本帧是否允许提交
    fn allow_present(&self, now: Instant) -> bool {
        self.min_interval.is_zero() || now.duration_since(self.last_present) >= self.min_interval
    }

    /// 丢帧判定：渲染段耗时超过 1.25 × 目标帧间隔。抽成纯函数供单测。
    fn is_miss(loop_us: u128, target_us: u128) -> bool {
        loop_us > target_us + target_us / 4
    }

    fn note_present(&mut self, now: Instant) {
        self.last_present = now;
    }

    fn tick(&mut self, loop_us: u128, rendered: bool) {
        if !self.enabled {
            return;
        }
        if rendered {
            self.frames += 1;
            let target = self.min_interval.as_micros().max(self.vblank_us);
            if Self::is_miss(loop_us, target) {
                self.missed += 1;
            }
        }
        if self.epoch.elapsed() < std::time::Duration::from_secs(1) {
            return;
        }
        let frames = self.frames;
        // 渲染帧很少 = 基本空闲 = 没有负载，视为干净窗口。否则降档之后
        // 一旦桌面变静就永远凑不满判定窗口，档位升不回来。
        let miss_rate = if frames >= 10 {
            self.missed as f64 / frames as f64
        } else {
            0.0
        };
        self.frames = 0;
        self.missed = 0;
        self.epoch = Instant::now();
        if Instant::now() < self.cooldown_until {
            return;
        }
        // 升到更低档位后稳住了 30 s：说明这次探针是成功的，恢复默认探针间隔
        if self.level > 0
            && Instant::now().duration_since(self.last_upgrade)
                >= std::time::Duration::from_secs(30)
        {
            self.upgrade_gate = std::time::Duration::from_secs(20);
        }
        // 电池供电时更保守：阈值收紧
        let (bad_th, good_th) = if on_battery() { (0.15, 0.03) } else { (0.25, 0.05) };
        if miss_rate > bad_th {
            self.bad_streak += 1;
            self.good_streak = 0;
        } else if miss_rate < good_th {
            self.good_streak += 1;
            self.bad_streak = 0;
        } else {
            self.good_streak = 0;
            self.bad_streak = 0;
        }
        if self.bad_streak >= 2 && frames >= 30 && self.level < 4 {
            self.set_level(self.level + 1, miss_rate, frames);
        } else if self.good_streak >= 4
            && self.level > 0
            && Instant::now().duration_since(self.last_downgrade) >= self.upgrade_gate
        {
            self.set_level(self.level - 1, miss_rate, frames);
        }
    }

    fn set_level(&mut self, level: u8, miss_rate: f64, frames: u64) {
        // 切换档位会重建金字塔，产生一次停顿；清空统计窗口，别让这次停顿
        // 被算成"还在丢帧"从而触发下一级降级。
        self.frames = 0;
        self.missed = 0;
        self.epoch = Instant::now();
        let up = level < self.level;
        if !up && Instant::now().duration_since(self.last_upgrade) < std::time::Duration::from_secs(10)
        {
            // 刚升上去就被打回：这次探针失败，下次探针间隔翻倍
            self.upgrade_gate = (self.upgrade_gate * 2).min(std::time::Duration::from_secs(300));
        }
        if up {
            self.last_upgrade = Instant::now();
        }
        self.level = level;
        self.bad_streak = 0;
        self.good_streak = 0;
        self.cooldown_until = Instant::now() + std::time::Duration::from_secs(3);
        if !up {
            self.last_downgrade = Instant::now();
        }
        self.min_interval = if level >= 2 {
            std::time::Duration::from_millis(1000 / 60)
        } else {
            std::time::Duration::from_secs(0)
        };
        eprintln!(
            "[adaptive] 质量档位{} -> {level}（1 秒窗口 {frames} 帧中丢 vblank {:.0}%）：{}",
            if up { "回升" } else { "降级" },
            miss_rate * 100.0,
            match level {
                0 => "全部效果开启，与默认视觉逐位一致",
                1 => "frost 降为 1/2 分辨率（模糊层差异 ≤3/255）",
                2 => "+ 帧率上限 60（抖动 -> 恒定 60 fps）",
                3 => "+ 关闭焦散与 Beer-Lambert 吸收",
                _ => "+ 关闭 RGB 色散",
            }
        );
    }

    fn frost_half(&self) -> bool {
        self.enabled && self.level >= 1
    }

    /// 把 level 3/4 的参数降级叠加到面板参数之上（level 0~2 时逐字段原样）
    fn apply(&self, p: &mut Params) {
        if !self.enabled || self.level < 3 {
            return;
        }
        p.caustic_gain = 0.0;
        p.absorption = 0.0;
        if self.level >= 4 {
            p.dispersion = 0.0;
        }
    }
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

    // 启动阶段进度日志：驱动异常时 D3D11CreateDevice / D3DCompile 可能长时间
    // 无响应，没有这些打点就无法判断卡在哪一步。
    let t_boot = Instant::now();
    eprintln!("[boot] 创建 D3D11 设备...");
    let gfx = match Device::create() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("D3D11 初始化失败: {e}");
            return;
        }
    };
    eprintln!("[boot] D3D11 设备就绪 (+{:.0} ms)", t_boot.elapsed().as_secs_f64() * 1000.0);
    if gfx.warp {
        eprintln!("[warn] 回退到 WARP 软件光栅");
    }
    // 着色器 / LUT / 采样器全进程一份，所有显示器画布共享。
    eprintln!("[boot] 编译着色器...");
    let glass = match GlassRenderer::new(&gfx.device) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("着色器初始化失败: {e}");
            return;
        }
    };
    eprintln!("[boot] 着色器就绪 (+{:.0} ms)", t_boot.elapsed().as_secs_f64() * 1000.0);

    eprintln!("[boot] 创建各屏画布与采集...");
    let mut monitors_rt: Vec<MonitorRt> = Vec::new();
    for info in infos {
        if let Some(mut m) = MonitorRt::create(&gfx, class_name, hinst, info) {
            if let Some(cap) = m.capture.as_ref() {
                eprintln!(
                    "[info] {} 采集后端 = {}",
                    m.info.name,
                    cap.backend_name()
                );
            }
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
    // GLASS_MONITORS=<n>：只用前 n 块屏。既是一个实用选项（把玻璃限制在主屏），
    // 也用于归因「第二块屏的采集/合成到底占多少」。
    if let Ok(n) = std::env::var("GLASS_MONITORS") {
        if let Ok(n) = n.parse::<usize>() {
            if n >= 1 && n < monitors_rt.len() {
                eprintln!("[info] GLASS_MONITORS={n}，只启用前 {n} 块屏");
                monitors_rt.truncate(n);
            }
        }
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
    // GLASS_FAR_GRID=1：4 块互不重叠、分开足够远的确定性布局，
    // 专供 tools/verify_mouse_isolation.py 做「拖一块，其余块逐位不变」的硬证明。
    // 为什么不能直接用 GLASS_MANY：那 20 块是密集重叠布局（步长 0.62w/0.60h），
    // 邻近余晖会沿重叠块的半透明阴影逐块传递，合法余晖与非法串扰无法区分，
    // 判定集合的传递闭包会吞掉全部 20 块，测试就失去判别力。
    // 光标落在块 0 内时，其余块的中心距都 > gate 支撑半径 3*mouse_radius=660，
    // 因此光标落在任一块上时，其余块必须逐位与光标在远处时完全一致。
    if std::env::var("GLASS_FAR_GRID").ok().as_deref() == Some("1") {
        manager.clear();
        // 2x2，块 500x340，横向间距 700 / 纵向间距 660（边到边）。
        // 判据要求：光标落在块 0 内部（偏心点）时，其余 3 块的**最近边缘**
        // 到光标的距离都必须 >= 900px，即高斯余晖 fall=exp(-d^2/2R^2) <= 2.4e-4，
        // 经 glint*hoverGain 两次相乘后 <=1e-8 -> 落到 8bit 必然为 0，可断言严格逐位一致。
        // （第一版布局纵向只隔 700px，fall=0.0064，实测仍有 2 个像素因舍入边界差 1 LSB。）
        let cells: [(f32, f32); 4] = [(600.0, 300.0), (1800.0, 300.0), (600.0, 1300.0), (1800.0, 1300.0)];
        for &(bx, by) in &cells {
            manager.add(bx.min((lw - 500.0).max(8.0)), by.min((lh - 340.0).max(8.0)), 500.0f32, 340.0f32);
        }
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

    // GLASS_CURSOR_AT=x,y：把初始光标位置固定下来（回归测试用）。
    // 配合 GLASS_SYNTH_CAP 可以在"光标在远处两个不同位置"之间做逐像素对比，
    // 用来证明非 hover 块不受光标移动影响。
    let cursor_start = match std::env::var("GLASS_CURSOR_AT")
        .ok()
        .and_then(|v| {
            let (x, y) = v.split_once(',')?;
            Some([x.parse::<f32>().ok()?, y.parse::<f32>().ok()?])
        }) {
        Some(c) => c,
        None => [
            monitors_rt[0].info.origin[0] as f32 + monitors_rt[0].info.size[0] as f32 * 0.5,
            monitors_rt[0].info.origin[1] as f32 + monitors_rt[0].info.size[1] as f32 * 0.5,
        ],
    };
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
        fs_check_at: Instant::now().checked_sub(std::time::Duration::from_secs(3600)).unwrap_or_else(std::time::Instant::now),
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
        apply_visibility(mon);
        if mon.ready {
            eprintln!("[info] {} 画布已显示 ({}x{})", mon.info.name, mon.info.size[0], mon.info.size[1]);
        }
    }

    // ---- 主循环：消息泵 + 事件驱动渲染（方案 10.4 / 12.4.1）----
    // 触发源为空时整帧跳过：不跑 Pass B、不 Present、不 Commit。
    let metrics_enabled = std::env::var("GLASS_METRICS").ok().as_deref() == Some("1");
    // 旧实现每帧 std::env::var 会加进程环境锁并分配 String，这里只读一次。
    let extreme_spec = std::env::var("GLASS_EXTREME_SPEC").ok().as_deref() == Some("1");
    // GLASS_SIM_BUSY=<ms>：给每个渲染帧注入一段确定性 CPU 忙等，用来验证自适应
    // 控制器的降档/回升闭环。真机上我们的渲染成本只有 0.16 ms/帧，远低于 8.33 ms
    // 预算，控制器在真实负载下（正确地）不会触发；没有这个开关就无法验证它还能不能工作。
    // 默认 0 = 关闭。注意它会同时抬高 CPU 占用，只在验证控制器时开。
    let sim_busy_us: u128 = std::env::var("GLASS_SIM_BUSY")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .map(|ms| (ms.max(0.0) * 1000.0) as u128)
        .unwrap_or(0);
    // GLASS_VERIFY_PASS_A=1：拍一张「脏矩形受限 Pass A」的帧后，立刻用同一张
    // capture 强制跑一次「全量 Pass A + 全量 Pass B」，输出 .full.bmp 对照图。
    let verify_pass_a = std::env::var("GLASS_VERIFY_PASS_A").ok().as_deref() == Some("1");
    let verify_shot_path = std::env::var("GLASS_INTERNAL_SHOT").ok();
    let mut verify_done = false;
    // 静止时的等待步长：8 ms ≈ 125 Hz，覆盖 120 Hz 显示器的一个刷新周期，
    // 保证鼠标悬停反馈延迟不超过半帧，同时静止桌面 CPU 趋近 0。
    const IDLE_WAIT_MS: u32 = 8;
    // 没有玻璃块的屏的采集保活间隔，以及判定"采集已陈旧"的阈值。
    let idle_capture_interval = std::time::Duration::from_millis(250);
    let mut capture_wait_ms = IDLE_WAIT_MS;
    let mut metrics = Metrics::new();
    // GLASS_SIM_BACKDROP=<W>x<H>@<HZ>：用固定尺寸、固定节拍的模拟桌面脏区取代
    // 真实 DDA 脏矩形。真实桌面的活动率会在 4~52 次/秒之间漂移，8 秒的测量
    // 窗口根本不可比；只有把负载确定性化，分段归因才有意义。
    let sim = std::env::var("GLASS_SIM_BACKDROP").ok().and_then(|v| {
        let (wh, sim_hz) = v.split_once('@')?;
        let (w, h) = wh.split_once('x')?;
        Some((
            w.parse::<i32>().ok()?,
            h.parse::<i32>().ok()?,
            sim_hz.parse::<f64>().ok()?.max(1.0),
        ))
    });
    let mut sim_next = Instant::now();
    if let Some((sw_, sh_, shz)) = sim {
        eprintln!("[info] 模拟桌面脏区 {sw_}x{sh_} @ {shz:.0} Hz（确定性负载）");
    }
    let hz = refresh_hz();
    let mut quality = QualityCtrl::new(hz);
    let mut frost_half_applied = false;
    eprintln!(
        "[info] 显示器刷新率 = {hz:.0} Hz，电源 = {}，自适应质量 = {}",
        if on_battery() { "电池" } else { "交流电" },
        if quality.enabled {
            "开启（默认；level 0 = 与未启用时逐位一致，持续丢帧才降档，GLASS_ADAPTIVE=0 可关）"
        } else {
            "关闭（GLASS_ADAPTIVE=0）"
        }
    );
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
        let msg_us = loop_start.elapsed().as_micros();
        let cap_start = Instant::now();

        if app.rebuild_monitors {
            app.rebuild_monitors = false;
            rebuild_monitors(&mut app);
        }

        // ---- 每屏采集 ----
        // DDA 的阻塞等待同时充当主循环唯一的休眠点（方案 12.4.1「以采集帧为
        // 主时钟」）：桌面有新内容立即返回，静止时在驱动里内核等待。
        let mut new_backdrop = false;
        // 哪些屏上有玻璃块（用上一帧的归属即可，跨屏是低频事件）
        let mut active = [false; 16];
        let mut overflow = false;
        for b in app.manager.blocks() {
            match active.get_mut(b.monitor) {
                Some(slot) => *slot = true,
                None => overflow = true,
            }
        }
        {
            let gfx = &app.gfx;
            for (mi, mon) in app.monitors.iter_mut().enumerate() {
                mon.new_backdrop = false;
                mon.backdrop_dirty = None;
                mon.copied_px = 0;
                // 没有玻璃块的屏：整条采集（AcquireNextFrame + 脏矩形 + 局部拷贝）
                // 都可以省掉，降到 4 Hz 保活即可。归因实测这一项单独值 +20% 帧率。
                let has_blocks = overflow || active.get(mi).copied().unwrap_or(true);
                let age = mon.last_capture.elapsed();
                if mon.capture.is_some()
                    && mon.ready
                    && !has_blocks
                    && age < idle_capture_interval
                {
                    continue;
                }
                // 只在「块刚落到这块屏」的那一帧强制阻塞取新帧，避免玻璃里
                // 闪出一帧旧桌面。其余情况：有块的屏用主循环决定的等待步长
                // （拖拽时 0，静止时 IDLE_WAIT_MS），没块的屏用 32 ms 保活。
                // 注意不能按"距上次采集多久"判断陈旧 —— 桌面安静时采集本来就稀疏，
                // 那样会让有块的屏也一直走 32 ms 阻塞，白降帧率。
                let just_activated = has_blocks && mon.idle_capturing;
                mon.idle_capturing = !has_blocks;
                let wait = if just_activated || !has_blocks {
                    32
                } else {
                    capture_wait_ms
                };
                let (outcome, dirty, copied, lost) = match mon.capture.as_mut() {
                    Some(cap) => {
                        let r = cap.try_capture(&gfx.device, &gfx.ctx, wait);
                        (r, cap.dirty_union(), cap.copied_px(), cap.access_lost())
                    }
                    // 没有采集源：到点再尝试重建（独占全屏 / 录屏软件抢占后恢复）
                    None => {
                        if Instant::now() >= mon.retry_at {
                            mon.retry_at =
                                Instant::now() + std::time::Duration::from_millis(500);
                            match Capture::for_monitor(&gfx.device, mon.info.hmonitor) {
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
                        mon.last_capture = Instant::now();
                        if !mon.ready && mon.compositor.capture_ready() {
                            mon.ready = true;
                            mon.hidden_capture = false;
                            apply_visibility(mon);
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
                            // 画面会冻结在最后一帧，因此采集一失效就隐藏画布，
                            // 恢复后再显示。
                            mon.hidden_capture = true;
                            apply_visibility(mon);
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

        let cap_us = cap_start.elapsed().as_micros();
        let prep_start = Instant::now();

        if let Some((sw_, sh_, shz)) = sim {
            let interval = std::time::Duration::from_secs_f64(1.0 / shz);
            let now = Instant::now();
            if now >= sim_next {
                // 脏区固定放在主屏正中，保证每帧 Pass A / Pass B 的工作量完全一致
                let (mo, ms) = {
                    let info = &app.monitors[0].info;
                    ([info.origin[0], info.origin[1]], [info.size[0], info.size[1]])
                };
                let cx = mo[0] + ms[0] / 2;
                let cy = mo[1] + ms[1] / 2;
                let rect = [
                    (cx - sw_ / 2).max(mo[0]),
                    (cy - sh_ / 2).max(mo[1]),
                    (cx + sw_ / 2).min(mo[0] + ms[0]),
                    (cy + sh_ / 2).min(mo[1] + ms[1]),
                ];
                for m in app.monitors.iter_mut() {
                    m.new_backdrop = true;
                    m.backdrop_dirty = Some(rect);
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

        // ---- 控制面板参数同步（每帧读取原子值，无锁） ----
        let previous_params = app.params.clone();
        app.shared.apply(&mut app.params);
        if extreme_spec {
            // Keep the requested extreme regression values after panel sync.
            app.params.spec_intensity = 4.0;
            app.params.spec_power = 1.0;
        }
        quality.apply(&mut app.params);
        let params_changed = app.params != previous_params;
        // level 1 的 frost 半分辨率档：切换会重建金字塔，只在档位真的变化时做
        let want_half = quality.frost_half();
        if want_half != frost_half_applied {
            for mon in app.monitors.iter_mut() {
                if let Err(e) = mon.compositor.set_frost_half(&app.gfx, want_half) {
                    eprintln!("[warn] {} 切换 frost 档位失败: {e}", mon.info.name);
                }
            }
            frost_half_applied = want_half;
        }
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
        // 与 glass_ps.hlsl 的 gate 支撑半径、Compositor 的脏区影响盒保持一致
        let influence = crate::glass::params::cursor_influence_px(app.params.mouse_radius) as f32
            + margin;
        let mut mouse_relevant = false;
        for mi in 0..app.monitors.len() {
            collect_monitor_blocks(&mut app, mi, params_changed);
            // 空屏画布隐藏（1 秒防抖）：见 MonitorRt::hidden_empty 的说明。
            {
                let empty = app.monitors[mi].blocks.is_empty();
                let mon = &mut app.monitors[mi];
                if empty {
                    match mon.empty_since {
                        Some(t) if t.elapsed() >= std::time::Duration::from_secs(1) => {
                            if !mon.hidden_empty {
                                mon.hidden_empty = true;
                                apply_visibility(mon);
                            }
                        }
                        Some(_) => {}
                        None => mon.empty_since = Some(Instant::now()),
                    }
                } else {
                    mon.empty_since = None;
                    if mon.hidden_empty {
                        mon.hidden_empty = false;
                        apply_visibility(mon);
                    }
                }
            }
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

        // 独占全屏应用检测（方案 R-16）：4 Hz 轮询，命中就隐藏该屏画布，
        // 避免全屏置顶画布破坏游戏的独占全屏与 MPO。
        if app.fs_check_at.elapsed() >= std::time::Duration::from_millis(250) {
            app.fs_check_at = Instant::now();
            let drag_active = app.drag.is_some();
            for mon in app.monitors.iter_mut() {
                // 拖拽中隐藏画布会释放 SetCapture，把拖拽状态机打断，
                // 因此拖拽期间推迟到抬起后再处理。
                let hit = !drag_active && fullscreen_app_on(mon.hwnd);
                if hit != mon.hidden_fs {
                    mon.hidden_fs = hit;
                    apply_visibility(mon);
                    if hit {
                        eprintln!("[info] {} 检测到独占全屏应用，暂时隐藏玻璃", mon.info.name);
                    } else {
                        eprintln!("[info] {} 独占全屏应用已退出，恢复玻璃", mon.info.name);
                    }
                }
            }
        }

        let prep_us = prep_start.elapsed().as_micros();

        // ---- 渲染门控（方案 12.4.1）：没有任何触发源就整帧跳过 ----
        let velocity_active =
            app.drag_velocity[0].abs() >= 0.01 || app.drag_velocity[1].abs() >= 0.01;
        let requested = RENDER_REQUEST.swap(false, std::sync::atomic::Ordering::Acquire);
        let fps_capped = !quality.allow_present(Instant::now());
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
            if !app.monitors[mi].ready {
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
            quality.note_present(Instant::now());
        }
        quality.tick(own_us, presented);
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
}

#[cfg(test)]
mod tests {
    use super::QualityCtrl;

    #[test]
    fn miss_threshold_is_125_percent_of_target() {
        let t = 8333u128; // 120 Hz
        assert!(!QualityCtrl::is_miss(t, t));
        assert!(!QualityCtrl::is_miss(t + t / 4, t)); // 恰好 1.25x 不算丢
        assert!(QualityCtrl::is_miss(t + t / 4 + 1, t));
        // 采集等待不得被计入：判据收到的必须是「我们自己的工作」
        assert!(!QualityCtrl::is_miss(90, t)); // 90 µs 的真实工作量绝不是丢帧
    }
    #[test]
    fn miss_threshold_respects_fps_cap_level() {
        let t = 16667u128; // level>=2 的 60 fps 上限
        assert!(!QualityCtrl::is_miss(16700, t));
        assert!(QualityCtrl::is_miss(21000, t));
    }
}
