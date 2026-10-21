//! 显示器枚举（方案 4.4 / 12.10）。
//!
//! 全程使用**物理像素**：进程已声明 PER_MONITOR_AWARE_V2，窗口尺寸、采集纹理
//! 尺寸与 UV 映射处在同一坐标系，避免 Windows 位图拉伸导致的模糊。
//! 每块屏幕各自记录 DPI，`scale = dpi / 主屏 dpi` 用于跨屏时保持逻辑尺寸不变。

use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, DEVMODEW, ENUM_CURRENT_SETTINGS,
    HDC, HMONITOR, MONITORINFOEXW,
};
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowLongPtrW, GetWindowRect, GWL_STYLE, WS_CAPTION, WS_POPUP,
    WS_THICKFRAME,
};

use super::window::is_own_window;

use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

/// MONITORINFOF_PRIMARY（windows 0.58 未导出该常量）
const MONITORINFOF_PRIMARY: u32 = 0x0000_0001;

#[derive(Clone, Debug)]
pub struct MonitorInfo {
    pub hmonitor: HMONITOR,
    pub name: String,
    /// 虚拟屏坐标系里的物理像素原点
    pub origin: [i32; 2],
    /// 物理像素尺寸（各屏可以完全不同）
    pub size: [i32; 2],
    pub dpi: f32,
    /// 相对主屏的 DPI 比例
    pub scale: f32,
    pub primary: bool,
}

unsafe extern "system" fn enum_proc(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let list = &mut *(data.0 as *mut Vec<HMONITOR>);
    list.push(hmonitor);
    TRUE
}

/// 枚举所有显示器：主屏排在第 0 位，其余按原点排序，保证索引稳定。
pub fn enumerate() -> Vec<MonitorInfo> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            HDC::default(),
            None,
            Some(enum_proc),
            LPARAM(&mut handles as *mut Vec<HMONITOR> as isize),
        );
    }
    let mut out: Vec<MonitorInfo> = handles
        .into_iter()
        .filter_map(|hmonitor| unsafe {
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            if !GetMonitorInfoW(hmonitor, &mut info as *mut MONITORINFOEXW as *mut _).as_bool() {
                return None;
            }
            let rect = info.monitorInfo.rcMonitor;
            let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
            let _ = GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
            let name_end = info
                .szDevice
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(info.szDevice.len());
            Some(MonitorInfo {
                hmonitor,
                name: String::from_utf16_lossy(&info.szDevice[..name_end]),
                origin: [rect.left, rect.top],
                size: [rect.right - rect.left, rect.bottom - rect.top],
                dpi: dpi_x.max(1) as f32,
                scale: 1.0,
                primary: (info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY) != 0,
            })
        })
        .collect();

    if out.is_empty() {
        return out;
    }
    // 主屏放第 0 位（块坐标系以主屏物理像素为逻辑单位）
    if let Some(i) = out.iter().position(|m| m.primary) {
        out.swap(0, i);
    } else {
        out.sort_by_key(|m| (m.origin[1], m.origin[0]));
    }
    let primary_dpi = out[0].dpi.max(1.0);
    for m in out.iter_mut() {
        m.scale = m.dpi / primary_dpi;
    }
    // 其余按位置排序，保证同一硬件布局下索引稳定
    let head = out.remove(0);
    out.sort_by_key(|m| (m.origin[1], m.origin[0]));
    out.insert(0, head);
    out
}

// ---------------------------------------------------------------------------
// 以下三项从 src/main.rs 搬迁而来（重构阶段 2，零行为变化）：
// 独占全屏检测、刷新率、电源状态。它们都是「显示器/桌面」层的 Win32 查询，
// 与本文件上方的枚举逻辑同属一层，故合并到 platform::display。
// ---------------------------------------------------------------------------

pub(crate) fn fullscreen_app_on(mon_hwnd: HWND) -> bool {
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

/// 显示器刷新率（方案 12.4.2 帧节奏 / 12.4.3 质量预算的基准）
pub(crate) fn refresh_hz() -> f32 {
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

pub(crate) fn on_battery() -> bool {
    unsafe {
        let mut st = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut st).is_ok() {
            // BatteryFlag 128 = 无电池（台式机）；ACLineStatus 0 = 未接电源
            return st.BatteryFlag != 128 && st.ACLineStatus == 0;
        }
    }
    false
}

/// 在 `(origin, size)` 列表里找最接近 `(x, y)` 的一块屏。
///
/// 与 [`index_at`] 同一套判据（到屏中心距离最近），但入参不含 `String`，
/// 因此调用方可以缓存一份轻量数组，避免每帧 clone `Vec<MonitorInfo>`。
#[must_use]
pub fn index_at_rects(rects: &[([i32; 2], [i32; 2])], x: f32, y: f32) -> usize {
    if rects.is_empty() {
        return 0;
    }
    let mut best = 0;
    let mut best_d = f32::MAX;
    for (i, &(origin, size)) in rects.iter().enumerate() {
        let w = size[0] as f32;
        let h = size[1] as f32;
        let cx = (x - (origin[0] as f32 + w * 0.5)).abs();
        let cy = (y - (origin[1] as f32 + h * 0.5)).abs();
        let d = cx * cx + cy * cy;
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::{index_at_rects, MonitorInfo};

    fn infos() -> Vec<MonitorInfo> {
        vec![
            MonitorInfo {
                hmonitor: windows::Win32::Graphics::Gdi::HMONITOR(std::ptr::null_mut()),
                name: "A".into(),
                origin: [0, 0],
                size: [1000, 800],
                dpi: 96.0,
                scale: 1.0,
                primary: true,
            },
            MonitorInfo {
                hmonitor: windows::Win32::Graphics::Gdi::HMONITOR(std::ptr::null_mut()),
                name: "B".into(),
                origin: [1000, 0],
                size: [1920, 1080],
                dpi: 192.0,
                scale: 2.0,
                primary: false,
            },
        ]
    }

    #[test]
    fn picks_the_nearest_monitor_by_center_distance() {
        let list = infos();
        let rects: Vec<([i32; 2], [i32; 2])> = list.iter().map(|m| (m.origin, m.size)).collect();
        assert_eq!(index_at_rects(&rects, 0.0, 0.0), 0, "A 屏左上角");
        assert_eq!(index_at_rects(&rects, 999.0, 799.0), 0, "A 屏右下角");
        assert_eq!(index_at_rects(&rects, 1500.0, 500.0), 1, "B 屏内部");
        // 屏外取最近的一块（MONITOR_DEFAULTTONEAREST 语义）
        assert_eq!(index_at_rects(&rects, -400.0, -300.0), 0);
        assert_eq!(index_at_rects(&rects, 5000.0, 5000.0), 1);
        // NaN 不能 panic，落到第 0 块即可
        assert_eq!(index_at_rects(&rects, f32::NAN, f32::NAN), 0);
    }

    #[test]
    fn empty_list_returns_zero() {
        assert_eq!(index_at_rects(&[], 100.0, 100.0), 0);
    }
}
