#![windows_subsystem = "console"]
//! liquid-glass —— Windows 桌面液态玻璃（严格按《折射渲染方案》v1.1 实现）
//!
//! 管线：
//!   DXGI Desktop Duplication 采集主显示器 -> 1 份共享 D3D11 纹理（单次 GPU copy）
//!   -> 全屏 dual-filter 模糊金字塔（4 级，一帧一次，20 块共享）
//!   -> 单全屏画布 + DirectComposition（每块 scissor 绘制，一次 Present / Commit）
//!   -> Snell 折射 + RGB 色散 + Fresnel 高光 + 焦散 + 鼠标点光 + 自适应染色
//!
//! 性能设计（方案第 7 / 12 章）：
//!   - 采集成本 O(屏幕)，与块数无关；
//!   - 模糊金字塔 O(屏幕)，与块数无关；
//!   - 块绘制只覆盖各自像素面积；
//!   - 单设备、单交换链、单 Present，20+ 块不增加合成开销。

mod capture;
mod gfx;
mod glass;
mod input;
mod ui;

use std::time::Instant;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
use windows::Win32::Graphics::Gdi::UpdateWindow;
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, GetMonitorInfoW, MonitorFromPoint, SetWindowRgn,
    GDI_REGION_TYPE, MONITORINFO, MONITOR_DEFAULTTOPRIMARY, RGN_OR,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

use capture::DdaCapture;
use gfx::{Compositor, Device};
use glass::{Manager, Params};
use input::{cursor_pos, DragKind, DragState};

/// 画布窗口句柄（供控制面板关闭时联动退出）
static CANVAS_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
pub fn canvas_hwnd() -> HWND {
    HWND(CANVAS_HWND.load(std::sync::atomic::Ordering::SeqCst) as *mut _)
}

/// 单画布模式：所有块共享一个 overlay 窗口与一个交换链
struct App {
    gfx: Device,
    compositor: Compositor,
    manager: Manager,
    params: Params,
    shared: &'static ui::SharedParams,
    hwnd: HWND,
    canvas_origin: [f32; 2],
    last_block_action: isize,
    drag: Option<DragState>,
    hover: Option<u32>,
    cursor: [f32; 2],
    press: f32,
    last_cursor: [f32; 2],
    last_drag_apply: f64,
    drag_velocity: [f32; 2],
    t0: Instant,
    frames: u64,
}

impl App {
    fn time(&self) -> f32 {
        self.t0.elapsed().as_secs_f32()
    }

    fn cursor_canvas(&self) -> [f32; 2] {
        [
            self.cursor[0] - self.canvas_origin[0],
            self.cursor[1] - self.canvas_origin[1],
        ]
    }
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
            app.shared.new_width.store(
                block.width.round() as isize,
                std::sync::atomic::Ordering::Relaxed,
            );
            app.shared.new_height.store(
                block.height.round() as isize,
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
                    let c = [
                        p.x as f32 - app.canvas_origin[0],
                        p.y as f32 - app.canvas_origin[1],
                    ];
                    if let Some(id) = app.manager.hit_test(c[0], c[1]) {
                        app.manager.bring_to_front(id, &app.params);
                        app.shared
                            .selected_id
                            .store(id as isize, std::sync::atomic::Ordering::Relaxed);
                        if let Some(b) = app.manager.get(id) {
                            app.shared.new_width.store(
                                b.width.round() as isize,
                                std::sync::atomic::Ordering::Relaxed,
                            );
                            app.shared.new_height.store(
                                b.height.round() as isize,
                                std::sync::atomic::Ordering::Relaxed,
                            );
                            app.shared
                                .selected_id
                                .store(id as isize, std::sync::atomic::Ordering::Relaxed);
                        }
                        if let Some(b) = app.manager.get(id) {
                            let near_corner =
                                c[0] > b.x + b.width - 22.0 && c[1] > b.y + b.height - 22.0;
                            let kind = if near_corner {
                                DragKind::ResizeSE
                            } else {
                                DragKind::Move
                            };
                            let origin = [b.x, b.y, b.width, b.height];
                            app.drag = Some(DragState::begin(
                                id,
                                kind,
                                c,
                                origin,
                                app.t0.elapsed().as_secs_f64(),
                            ));
                        }
                        app.press = 1.0;
                        let _ = SetCapture(hwnd);
                    }
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                if let Some(app) = app_of(hwnd) {
                    let p = cursor_pos();
                    app.cursor = [p.x as f32, p.y as f32];
                    let c = app.cursor_canvas();
                    app.hover = app.manager.hit_test(c[0], c[1]);
                }
                LRESULT(0)
            }
            WM_NCHITTEST => {
                if let Some(app) = app_of(hwnd) {
                    let p = cursor_pos();
                    let c = [
                        p.x as f32 - app.canvas_origin[0],
                        p.y as f32 - app.canvas_origin[1],
                    ];
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
                            app.shared.new_width.store(
                                b.width.round() as isize,
                                std::sync::atomic::Ordering::Relaxed,
                            );
                            app.shared.new_height.store(
                                b.height.round() as isize,
                                std::sync::atomic::Ordering::Relaxed,
                            );
                            app.shared
                                .selected_id
                                .store(d.id as isize, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                }
                let _ = ReleaseCapture();
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            WM_KEYDOWN => {
                if wp.0 == 0x1B {
                    let _ = DestroyWindow(hwnd);
                    return LRESULT(0);
                }
                if let Some(app) = app_of(hwnd) {
                    // 键盘快捷：+ / - 增删一块（无控制面板时的最小可用路径）
                    match wp.0 as u32 {
                        0xBB => {
                            // VK_OEM_PLUS
                            let i = app.manager.len();
                            let (w, h) = screen_logical();
                            app.manager.add_auto(i, 320.0, 200.0, w, h);
                            sync_selection(app);
                        }
                        0xBD => {
                            // VK_OEM_MINUS
                            let id = app.manager.blocks().last().map(|b| b.id);
                            if let Some(id) = id {
                                app.manager.remove(id);
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

fn screen_logical() -> (f32, f32) {
    let w = unsafe { GetSystemMetrics(SM_CXSCREEN) } as f32;
    let h = unsafe { GetSystemMetrics(SM_CYSCREEN) } as f32;
    (w, h)
}

/// Restrict the canvas to the union of the blocks and their shadow margins.
/// SetWindowRgn takes ownership of a successful region assignment.
fn update_canvas_region(app: &App) {
    let width = app.compositor.width.max(0);
    let height = app.compositor.height.max(0);
    let margin = app.params.shadow_margin.max(0.0);

    unsafe {
        let mut region = None;
        for block in app.manager.blocks() {
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
            if !empty.0.is_null() && SetWindowRgn(app.hwnd, empty, true) == 0 {
                let _ = DeleteObject(empty);
            }
            return;
        };

        if SetWindowRgn(app.hwnd, region, true) == 0 {
            let _ = DeleteObject(region);
        }
    }
}

fn monitor_rect() -> (i32, i32, i32, i32) {
    let pt = POINT { x: 0, y: 0 };
    let mon = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTOPRIMARY) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        let _ = GetMonitorInfoW(mon, &mut info);
    }
    let r = info.rcMonitor;
    (r.left, r.top, r.right - r.left, r.bottom - r.top)
}

fn main() {
    unsafe {
        if let Err(e) = RoInitialize(RO_INIT_MULTITHREADED) {
            eprintln!("WinRT 初始化失败: {e}");
            return;
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let (mx, my, mw, mh) = monitor_rect();
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

    // 单全屏画布窗口：WS_EX_NOREDIRECTIONBITMAP + WS_EX_TOPMOST + 点击穿透
    let hwnd: HWND = unsafe {
        CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW,
            class_name,
            windows::core::w!("liquid glass canvas"),
            WS_POPUP,
            mx,
            my,
            mw,
            mh,
            HWND::default(),
            None,
            hinst,
            None,
        )
    }
    .unwrap_or_default();
    if hwnd.0.is_null() {
        eprintln!("CreateWindowExW 失败");
        return;
    }

    // 让 DComp 接管整窗（无重定向位图）
    unsafe {
        let m = MARGINS {
            cxLeftWidth: -1,
            cxRightWidth: -1,
            cyTopHeight: -1,
            cyBottomHeight: -1,
        };
        let _ = DwmExtendFrameIntoClientArea(hwnd, &m);
    }

    // 自反馈根治：默认始终把玻璃窗口从 WGC 采集中剔除。
    // DDA 路径仍需把 overlay 排除在捕获之外，否则玻璃会逐帧递归采入自身，
    // 形成与用户截图一致的持续变黑/残影。与参考 examples 保持一致。
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowDisplayAffinity, WINDOW_DISPLAY_AFFINITY,
        };
        const WDA_EXCLUDEFROMCAPTURE: u32 = 0x0000_0011;
        if let Err(e) = SetWindowDisplayAffinity(
            hwnd,
            WINDOW_DISPLAY_AFFINITY(WDA_EXCLUDEFROMCAPTURE),
        ) {
            eprintln!("[warn] SetWindowDisplayAffinity: {e:?}");
        }
    }

    // ---- D3D11 设备 ----
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

    // ---- 交换链 + DComp ----
    let compositor = match Compositor::new(&gfx, hwnd, mw, mh) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("合成器初始化失败: {e}");
            return;
        }
    };

    // ---- DXGI Desktop Duplication 采集 ----
    // 与理想 examples 相同的输入源。它不会把 WDA 排除窗口写成黑色占位，
    // 也不会采集 DComp overlay，因此可持续更新而无自反馈。
    let mut capture = match DdaCapture::new(&gfx.device) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("DDA 采集初始化失败: {e}");
            return;
        }
    };

    // 默认先展示一块完整玻璃；需要压力测试时设置 GLASS_MANY=1。
    let mut manager = Manager::default();
    let (lw, lh) = screen_logical();
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

    // 中文控制面板（Win32 原生 slider）
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

    let mut app = App {
        gfx,
        compositor,
        manager,
        params: Params::default(),
        shared,
        hwnd,
        canvas_origin: [mx as f32, my as f32],
        last_block_action: 0,
        drag: None,
        hover: None,
        cursor: [mx as f32 + mw as f32 * 0.5, my as f32 + mh as f32 * 0.5],
        press: 0.0,
        last_cursor: [0.0, 0.0],
        last_drag_apply: 0.0,
        drag_velocity: [0.0, 0.0],
        t0: Instant::now(),
        frames: 0,
    };
    app.params.sanitize();
    sync_selection(&app);
    let _ = app.compositor.glass.update_lut(&app.gfx.ctx, &app.params);

    // 确保金字塔已按画布尺寸初始化（不依赖 WGC 帧尺寸）
    if let Err(e) = app.compositor.resize(&app.gfx, mw, mh) {
        eprintln!("初始化金字塔失败: {e}");
        return;
    }

    CANVAS_HWND.store(hwnd.0 as isize, std::sync::atomic::Ordering::SeqCst);
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, &mut app as *mut App as isize);
    }
    update_canvas_region(&app);

    // 参考 liquid-glass-rs：overlay 显示前先捕获一张完整桌面基底。
    // 等到首帧建立共享模糊资源后再 ShowWindow，从根源消除初始黑帧。
    let capture_deadline = Instant::now() + std::time::Duration::from_secs(5);
    while !app.compositor.capture_ready() && Instant::now() < capture_deadline {
        match capture.try_capture(&app.gfx.device, &app.gfx.ctx) {
            Ok(Some((tex, w, h))) => {
                if app.compositor.width != w || app.compositor.height != h {
                    let _ = app.compositor.resize(&app.gfx, w, h);
                }
                let _ = app.compositor.set_capture(&app.gfx, tex);
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(8)),
            Err(e) => {
                eprintln!("DDA 首帧: {e}");
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
        }
    }
    if !app.compositor.capture_ready() {
        eprintln!("WGC 首帧超时，拒绝显示黑色玻璃");
        return;
    }
    // 先渲染/提交一帧完整玻璃，再显示窗口，避免闪黑。
    let _ = app.compositor.render(
        &app.gfx, &app.manager, &app.params, app.cursor, app.canvas_origin,
        app.drag_velocity, app.time(), app.press, app.hover,
    );
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = UpdateWindow(hwnd);
    }

    // ---- 主循环：消息泵 + 每帧渲染 ----
    let mut msg = MSG::default();
    loop {
        while unsafe { PeekMessageW(&mut msg, HWND::default(), 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == WM_QUIT {
                return;
            }
            let _ = unsafe { TranslateMessage(&msg) };
            unsafe { DispatchMessageW(&msg) };
        }

        // DDA 与参考 examples 相同：持续复制最新桌面帧。DComp overlay 不会进入
        // duplication，因此不存在 WGC+WDA 的黑色占位，也没有递归自反馈。
        match capture.try_capture(&app.gfx.device, &app.gfx.ctx) {
            Ok(Some((tex, w, h))) => {
                if app.compositor.width != w || app.compositor.height != h {
                    if let Err(e) = app.compositor.resize(&app.gfx, w, h) {
                        eprintln!("resize: {e}");
                    }
                }
                if let Err(e) = app.compositor.set_capture(&app.gfx, tex) {
                    eprintln!("set_capture: {e}");
                }
            }
            Ok(None) => {}
            Err(e) => eprintln!("DDA capture: {e}"),
        }

        // ---- 控制面板参数同步（每帧读取原子值，无锁） ----
        let previous_params = app.params.clone();
        app.shared.apply(&mut app.params);
        if app.params != previous_params {
            if (app.params.ior - previous_params.ior).abs() > f32::EPSILON {
                if let Err(e) = app.compositor.glass.update_lut(&app.gfx.ctx, &app.params) {
                    eprintln!("update LUT: {e}");
                }
            }
            // 参数变化只影响 Pass B 的 shader 常量，不需要重算背景金字塔。
        }
        // 面板「宽度/高度」作用于当前选中（或最近操作）的块；
        // 「圆角半径」滑杆作为全局默认，未单独调整过的块跟随其变化。
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
        if let Some(id) = (target_id >= 0 && !dragging).then_some(target_id as u32) {
            if let Some(b) = app.manager.get_mut(id) {
                if target_w >= 24.0 && b.width != target_w {
                    b.width = target_w;
                }
                if target_h >= 24.0 && b.height != target_h {
                    b.height = target_h;
                }
                b.radius = radius_slider.min(0.5 * b.width.min(b.height));
                b.clamp();
            }
        } else {
            for b in app.manager.blocks_mut().iter_mut() {
                b.radius = radius_slider.min(0.5 * b.width.min(b.height));
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
            match action {
                1 => {
                    let i = app.manager.len();
                    let (sw, sh) = screen_logical();
                    let id = app.manager.add_auto(i, nw, nh, sw, sh);
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
                    }
                    sync_selection(&app);
                }
                3 => {
                    app.manager.clear();
                    app.shared
                        .selected_id
                        .store(-1, std::sync::atomic::Ordering::Relaxed);
                }
                4 => {
                    let mut last = -1isize;
                    for _ in 0..20 {
                        let i = app.manager.len();
                        let (sw, sh) = screen_logical();
                        let id = app.manager.add_auto(i, nw, nh, sw, sh);
                        last = id as isize;
                    }
                    app.shared
                        .selected_id
                        .store(last, std::sync::atomic::Ordering::Relaxed);
                }
                _ => {}
            }
            app.last_block_action = action;
            sync_selection(&app);
        }

        // 鼠标状态（每帧轮询，无 hook；方案 6.3）
        let p = cursor_pos();
        app.cursor = [p.x as f32, p.y as f32];
        let cursor_canvas = app.cursor_canvas();
        app.hover = match app.drag.as_ref() {
            Some(d) => Some(d.id),
            None => app.manager.hit_test(cursor_canvas[0], cursor_canvas[1]),
        };

        // 拖拽：每帧应用一次（方案 12.2.3 输入合并）
        if let Some(drag) = app.drag.as_mut() {
            let now = app.t0.elapsed().as_secs_f64();
            let r = drag.apply(cursor_canvas, now);
            app.drag_velocity = drag.velocity;
            if let Some(b) = app.manager.get_mut(drag.id) {
                b.x = r[0];
                b.y = r[1];
                b.width = r[2];
                b.height = r[3];
                b.radius = b.radius.min(0.5 * b.width.min(b.height));
                b.clamp();
            }
        } else {
            app.drag_velocity[0] *= 0.85;
            app.drag_velocity[1] *= 0.85;
        }
        update_canvas_region(&app);

        // 鼠标点击穿透：光标在某块内 -> 取消穿透；否则恢复穿透（方案 12.1.4）
        let inside = app
            .manager
            .hit_test(cursor_canvas[0], cursor_canvas[1])
            .is_some()
            || app.drag.is_some();
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
                SetWindowLongPtrW(
                    hwnd,
                    GWL_EXSTYLE,
                    (style | WS_EX_TRANSPARENT.0 as i64) as isize,
                );
            }
        }

        // 始终渲染：即使无 WGC 帧也用 fallback 纹理显示玻璃轮廓
        {
            let t = app.time();
            if let Err(e) = app.compositor.render(
                &app.gfx,
                &app.manager,
                &app.params,
                app.cursor,
                app.canvas_origin,
                app.drag_velocity,
                t,
                app.press,
                app.hover,
            ) {
                eprintln!("render: {e}");
            }
        }

        app.frames += 1;
        // 事件驱动：桌面静止 + 鼠标不动 + 无拖拽时可以睡一会儿
        let idle = app.drag.is_none()
            && app.drag_velocity[0].abs() < 0.01
            && app.drag_velocity[1].abs() < 0.01;
        if idle {
            std::thread::sleep(std::time::Duration::from_millis(4));
        }
        let _ = app;
        let _ = app.hwnd;
        let _ = app.last_cursor;
        let _ = app.last_drag_apply;
        let _ = &mut app;
    }
}
