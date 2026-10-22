//! Win32 窗口层：窗口类 / 画布创建 / 窗口过程 / 命中测试 / 窗口区域 / 可见性。
//!
//! 这一层承载了三条实测得来的硬约束，改之前务必读注释：
//! 1. **拖拽期零 `SetWindowRgn`**：按下时一次性把区域扩到整窗，抬起后收紧。
//!    旧实现每帧重建 HRGN + SetWindowRgn，会让 DWM 反复重裁剪整个 overlay，
//!    是 100~280 ms 主循环长尾的主要来源。
//! 2. **空区域必须真的空**：`SetWindowRgn(hwnd, NULL)` 会恢复整窗矩形、把桌面重新挡死
//!    （`WS_EX_TRANSPARENT` 不是命中测试契约），因此无块时要设一个 0×0 的 HRGN。
//! 3. **命中测试用 `WM_NCHITTEST` 返回 `HTTRANSPARENT`**，配合逐帧切换 `WS_EX_TRANSPARENT`，
//!    做到「块内可交互、块外完全穿透」。

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, GDI_REGION_TYPE, RGN_OR,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::api::config::WindowLevel;
use crate::api::event::EngineEvent;
use crate::api::handle::Command;
use crate::core::handle::ResizeEdges;
use crate::error::{Error, ErrorKind, Result};
use crate::input::{DragSession, MoveState, ResizeState};
use crate::platform::{display::MonitorInfo, level};
use crate::runtime::{app::App, monitor::MonitorRt};

/// 画布窗口类名（窄字符）。
pub(crate) const CANVAS_CLASS_NAME: &str = "LiquidVerreCanvas";

/// 画布窗口类名（宽字符，含结尾 0）。
///
/// 由 [`CANVAS_CLASS_NAME`] 在**编译期**展开，保证「注册用的名字」与
/// `is_own_window` 比较用的名字是同一个来源，不会漂移。
const CANVAS_CLASS_NAME_W: &[u16] = &{
    let bytes = CANVAS_CLASS_NAME.as_bytes();
    let mut buf = [0u16; CANVAS_CLASS_NAME.len() + 1];
    let mut i = 0;
    while i < bytes.len() {
        buf[i] = bytes[i] as u16;
        i += 1;
    }
    buf
};

/// 类名的 `PCWSTR` 形式。
#[must_use]
pub(crate) fn class_name_w() -> PCWSTR {
    PCWSTR(CANVAS_CLASS_NAME_W.as_ptr())
}

/// 取模块句柄。
#[must_use]
pub(crate) fn module_instance() -> HINSTANCE {
    let hmod = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
    HINSTANCE(hmod.0)
}

/// 注册画布窗口类（进程内一次）。
///
/// 已经注册过（同进程第二个 `Engine`）时返回成功而不是报错 —— 类是进程级资源，
/// 两个引擎共用同一个类完全没问题。
pub(crate) fn register_class(hinst: HINSTANCE) -> Result<PCWSTR> {
    let name = class_name_w();
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_CLASSDC,
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinst,
        lpszClassName: name,
        ..Default::default()
    };
    if unsafe { RegisterClassExW(&wc) } == 0 {
        let err = windows::core::Error::from_win32();
        // ERROR_CLASS_ALREADY_EXISTS = 1410
        if err.code().0 as u32 != 1410 {
            return Err(Error::with_source(
                ErrorKind::WindowCreation,
                "RegisterClassExW 失败",
                err,
            ));
        }
    }
    Ok(name)
}

/// 创建一块显示器的全屏画布窗口。
pub(crate) fn create_canvas(
    _gfx: &crate::gfx::Device,
    info: &MonitorInfo,
    lvl: WindowLevel,
) -> Result<HWND> {
    let hinst = module_instance();
    let class_name = class_name_w();
    // 初始扩展样式按层级决定要不要 TOPMOST；点击穿透由主循环逐帧维护。
    let mut ex =
        WS_EX_NOREDIRECTIONBITMAP | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
    if lvl.is_topmost() {
        ex |= WS_EX_TOPMOST;
    }
    let hwnd = unsafe {
        CreateWindowExW(
            ex,
            class_name,
            windows::core::w!("liquid verre canvas"),
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
    .map_err(|e| Error::with_source(ErrorKind::WindowCreation, "CreateWindowExW(画布) 失败", e))?;
    if hwnd.0.is_null() {
        return Err(Error::new(
            ErrorKind::WindowCreation,
            "CreateWindowExW 返回空 HWND",
        ));
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
            log::warn!("SetWindowDisplayAffinity({}): {e:?}", info.name);
        }
    }
    Ok(hwnd)
}

/// 对全部画布应用层级（需求 F4）。
pub(crate) fn apply_level(monitors: &[MonitorRt], lvl: WindowLevel) {
    for mon in monitors {
        level::apply(
            mon.hwnd,
            lvl,
            mon.ready && !mon.hidden_fs && !mon.hidden_capture && !mon.hidden_empty,
        );
    }
}

/// 取与窗口关联的 `App`；未设置时返回 `None`。
///
/// # Safety
/// `hwnd` 的 `GWLP_USERDATA` 必须要么为 0，要么指向一个存活的 `App`。
/// 这个不变量由 `runtime` 在创建 / 重建 / 销毁画布时维护（销毁前先清零）。
pub(crate) unsafe fn app_of(hwnd: HWND) -> Option<&'static mut App> {
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
                on_button_down(hwnd);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                // 只记光标位置；hover / 把手判定在主循环里每帧做一次
                // （方案 12.2.3 输入合并：1000 Hz 消息 → 一帧一次）。
                if let Some(app) = app_of(hwnd) {
                    app.cursor = app.read_cursor();
                }
                LRESULT(0)
            }
            WM_NCHITTEST => {
                let Some(app) = app_of(hwnd) else {
                    return LRESULT(HTTRANSPARENT as isize);
                };
                let c = app.read_cursor();
                let hit = app.drag.is_some()
                    || app.manager.hit_test(c[0], c[1]).is_some()
                    || app.manager.hit_resize(c[0], c[1]).is_some();
                LRESULT(if hit {
                    HTCLIENT as isize
                } else {
                    HTTRANSPARENT as isize
                })
            }
            WM_SETCURSOR => {
                // 只在客户区（= 命中玻璃块）改光标；其余情况交回默认处理，
                // 否则会覆盖宿主应用的光标。
                let hit_test_code = (lp.0 as u32) & 0xffff;
                if hit_test_code == HTCLIENT {
                    if let Some(app) = app_of(hwnd) {
                        if let Some(id) = app.hover_handle.cursor_id() {
                            let cursor = LoadCursorW(None, PCWSTR(id as usize as *const u16))
                                .unwrap_or_default();
                            if !cursor.0.is_null() {
                                SetCursor(cursor);
                                return LRESULT(1);
                            }
                        }
                    }
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_LBUTTONUP => {
                on_button_up(hwnd);
                LRESULT(0)
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED => {
                // 分辨率 / 缩放 / 拔插显示器：重建每屏画布与采集。
                if let Some(app) = app_of(hwnd) {
                    app.rebuild_monitors = true;
                }
                request_render(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            WM_KEYDOWN => {
                // 画布带 WS_EX_NOACTIVATE，正常情况下拿不到键盘焦点；
                // 真的拿到时 ESC 请求引擎退出（与重构前的行为一致）。
                if wp.0 == 0x1B {
                    if let Some(app) = app_of(hwnd) {
                        app.shared.push(Command::Shutdown);
                    }
                    return LRESULT(0);
                }
                request_render(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

unsafe fn request_render(hwnd: HWND) {
    if let Some(app) = app_of(hwnd) {
        app.shared.request_render();
    }
}

/// 按下：先判缩放把手（角优先于边），再判整块移动。
unsafe fn on_button_down(hwnd: HWND) {
    let Some(app) = app_of(hwnd) else { return };
    let c = app.read_cursor();
    app.cursor = c;
    let handle = app.manager.hit_resize(c[0], c[1]);
    let hit = match handle {
        Some((id, _)) => Some((id, handle.map(|(_, e)| e))),
        None => app.manager.hit_test(c[0], c[1]).map(|id| (id, None)),
    };
    let Some((id, edges)) = hit else { return };

    // 不可移动的块：只有缩放把手能开始会话；按内部直接放弃（不 SetCapture、
    // 不发 MoveStarted），但点击抬层仍然生效（见下）。
    if edges.is_none() {
        if let Some(b) = app.manager.get(id) {
            if !b.movable {
                if app.manager.bring_to_front(id) {
                    app.geometry_changed = true;
                }
                if app.level.raises_on_click() {
                    level::raise_on_click(hwnd);
                }
                app.shared.request_render();
                return;
            }
        }
    }

    // 点击到最前：同层内抬到最上面；WindowLevel::ClickToFront 还要抬画布窗口。
    if app.manager.bring_to_front(id) {
        app.geometry_changed = true;
    }
    if app.level.raises_on_click() {
        level::raise_on_click(hwnd);
    }

    let Some(b) = app.manager.get(id) else { return };
    let s = b.scale.max(1e-3);
    // origin 的 w/h 存**逻辑单位**，跨屏拖拽 / 缩放才不会被按下那一刻的像素尺寸锁死。
    let origin = [b.x, b.y, b.width / s, b.height / s];
    let min_size = [b.resize.min_width, b.resize.min_height];
    let now = app.t0.elapsed().as_secs_f64();
    app.min_drag_size = min_size;
    app.drag = Some(match edges {
        Some(e) => DragSession::Resize(ResizeState::begin(id, e, c, origin, now)),
        None => DragSession::Move(MoveState::begin(id, c, origin, now)),
    });
    app.hover = Some(id);
    app.hover_handle = edges.unwrap_or(ResizeEdges::NONE);
    app.press = 1.0;
    let _ = SetCapture(hwnd);
    app.shared.request_render();
    match edges {
        Some(e) => {
            if let Some(handle) = e.public() {
                app.shared.emit(EngineEvent::ResizeStarted { id, handle });
            }
        }
        None => app.shared.emit(EngineEvent::MoveStarted { id }),
    }
}

/// 抬起：结束会话并上抛结果矩形（逻辑像素）。
unsafe fn on_button_up(hwnd: HWND) {
    if let Some(app) = app_of(hwnd) {
        app.press = 0.0;
        if let Some(session) = app.drag.take() {
            let id = session.id();
            let resized = session.is_resize();
            if let Some(b) = app.manager.get(id) {
                let s = b.scale.max(1e-3);
                let rect = [b.x, b.y, b.width / s, b.height / s];
                app.shared.emit(if resized {
                    EngineEvent::ResizeEnded { id, rect }
                } else {
                    EngineEvent::MoveEnded { id, rect }
                });
            }
            app.hover_handle = ResizeEdges::NONE;
            app.geometry_changed = true;
            for m in app.monitors.iter_mut() {
                m.region_dirty = true;
            }
        }
    }
    let _ = ReleaseCapture();
    request_render(hwnd);
}

/// 把画布窗口区域限制成「所有块 ⊕ 外扩量」的并集。
///
/// `SetWindowRgn` 会接管成功赋值的区域对象所有权。
pub(crate) fn update_region(mon: &MonitorRt, margin: f32) {
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
            // 空区域必须真的空：传 NULL 会恢复整窗矩形、把桌面重新挡死，
            // 因为 WS_EX_TRANSPARENT 不是命中测试契约。
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
///
/// 块外内容始终是全透明，扩大区域不改变任何像素；命中测试仍由 `WM_NCHITTEST` 的
/// `HTTRANSPARENT` 负责穿透。换来的是拖拽全程 0 次 `SetWindowRgn`。
pub(crate) fn set_region_full(mon: &MonitorRt) {
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

/// 画布可见性 = 采集就绪 且 未被独占全屏压制 且 本屏非空。
pub(crate) fn apply_visibility(mon: &MonitorRt) {
    unsafe {
        if mon.ready && !mon.hidden_fs && !mon.hidden_capture && !mon.hidden_empty {
            mon.show();
        } else {
            let _ = ShowWindow(mon.hwnd, SW_HIDE);
        }
    }
}

/// 点击穿透切换（方案 12.1.4）：光标在本屏某块内 → 取消穿透；否则恢复。
pub(crate) fn set_click_through(hwnd: HWND, inside: bool) {
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
}

/// 该 HWND 是不是我们自己的画布（类名固定）。
///
/// 用来避免把自家 overlay 误判成独占全屏应用 —— 画布本身就是 `WS_POPUP` + 铺满显示器，
/// 完全符合「全屏」的形状判据。
///
/// ⚠ 误判的后果被 A（`hidden_fs` 停产门）放大了：把**自家画布**认成「独占全屏应用」
/// 就会整屏停采 + 停产，玻璃直接消失，而且不会报成我们的错误。所以纯比较逻辑
/// 抽成 [`class_name_is_canvas`] 单独测 —— 真实 HWND 在 CI 里造不出来。
#[must_use]
pub(crate) fn is_own_window(hwnd: HWND) -> bool {
    if hwnd.0.is_null() {
        return false;
    }
    let mut buf = [0u16; 64];
    // `GetClassNameW` 返回**写入的字符数**（不含结尾 0），失败返回 0，两者都 <= buf.len()。
    // 仍然 clamp 一次：本函数现在也被 `platform::occlusion` 的 `EnumWindows` 回调以 8 Hz
    // 对桌面上**每个**窗口调用，而回调里 panic 穿 FFI + release profile 的 `panic = "abort"`
    // 会直接带走整个渲染进程 ⇒「切片不越界」必须是结构性质，不是对 Win32 返回值的信任。
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    class_name_is_canvas(&buf, n)
}

/// 类名比较的纯逻辑：`GetClassNameW` 写进 `buf` 的前 `n` 个宽字符，是否恰好是我们的画布类名。
///
/// **零分配**：直接比宽字符常量，不再 `String::from_utf16_lossy`（那是一次堆分配，
/// 而任务硬约束要求「`EnumWindows` 回调里不分配 `String` / `Vec`」；本函数被那条回调
/// 对桌面上每个窗口调用一次，4 Hz 的 `fullscreen_app_on` 也走它）。
/// 判据来源仍然是同一个 [`CANVAS_CLASS_NAME`]（编译期展开成 `CANVAS_CLASS_NAME_W`，
/// 单一来源，见其注释）。
///
/// `n` 是 `GetClassNameW` 的原始 `i32` 返回值：负数（不该出现）按 0 处理，超过 `buf`
/// 长度按 `buf.len()` 截断 —— 两条都用 saturating/clamp 表达，不留越界切片的余地。
#[must_use]
fn class_name_is_canvas(buf: &[u16], n: i32) -> bool {
    let n = (n.max(0) as usize).min(buf.len());
    // 两者的长度约定差一个结尾 0：`CANVAS_CLASS_NAME_W` **含**它，
    // 而 `GetClassNameW` 的计数**不含**，所以只比 len-1 那一段。
    let expect = &CANVAS_CLASS_NAME_W[..CANVAS_CLASS_NAME_W.len().saturating_sub(1)];
    &buf[..n] == expect
}

#[cfg(test)]
mod tests {
    use super::{class_name_is_canvas, CANVAS_CLASS_NAME, CANVAS_CLASS_NAME_W};

    #[test]
    fn class_name_is_canvas_accepts_only_the_exact_name() {
        // 这条判据错一次的后果：`fullscreen_app_on` 把自家画布当成独占全屏应用
        // ⇒ `hidden_fs` ⇒ 整屏停采停产 ⇒ 玻璃永久消失（A 放大了误判的代价）。
        let name = &CANVAS_CLASS_NAME_W[..CANVAS_CLASS_NAME_W.len() - 1];
        let mut buf = [0u16; 64];
        buf[..name.len()].copy_from_slice(name);
        assert!(
            class_name_is_canvas(&buf, name.len() as i32),
            "精确类名必须认成自家画布"
        );
        // 少一个字符 / 多一个结尾 0 / 空串 / 别的窗口类，一律不是自家画布
        assert!(!class_name_is_canvas(&buf, name.len() as i32 - 1));
        assert!(!class_name_is_canvas(&buf, name.len() as i32 + 1));
        assert!(!class_name_is_canvas(&buf, 0));
        let mut other = [0u16; 64];
        let o: Vec<u16> = "Progman".encode_utf16().collect();
        other[..o.len()].copy_from_slice(&o);
        assert!(!class_name_is_canvas(&other, o.len() as i32));
        // Win32 不该给出这两种返回值，但「不可能 panic」必须是结构性质：
        // 负数按 0 处理，超长按 buf.len() 截断，两者都只影响结论不影响安全性。
        assert!(!class_name_is_canvas(&buf, -1));
        assert!(!class_name_is_canvas(&buf, i32::MIN));
        assert!(!class_name_is_canvas(&buf, i32::MAX));
    }

    #[test]
    fn wide_class_name_matches_narrow_and_is_null_terminated() {
        let narrow: Vec<u16> = CANVAS_CLASS_NAME.encode_utf16().collect();
        assert_eq!(CANVAS_CLASS_NAME_W.len(), narrow.len() + 1);
        assert_eq!(&CANVAS_CLASS_NAME_W[..narrow.len()], narrow.as_slice());
        assert_eq!(
            CANVAS_CLASS_NAME_W[narrow.len()],
            0,
            "必须以 0 结尾，RegisterClassExW 才认"
        );
    }

    #[test]
    fn class_name_is_not_the_legacy_one() {
        // 重构前叫 LiquidGlassCanvas；改名后 is_own_window 的判据必须同步
        assert_eq!(CANVAS_CLASS_NAME, "LiquidVerreCanvas");
        assert!(!CANVAS_CLASS_NAME.contains("Glass"));
    }
}
