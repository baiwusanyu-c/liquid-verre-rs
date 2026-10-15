//! 显示器枚举（方案 4.4 / 12.10）。
//!
//! 全程使用**物理像素**：进程已声明 PER_MONITOR_AWARE_V2，窗口尺寸、采集纹理
//! 尺寸与 UV 映射处在同一坐标系，避免 Windows 位图拉伸导致的模糊。
//! 每块屏幕各自记录 DPI，`scale = dpi / 主屏 dpi` 用于跨屏时保持逻辑尺寸不变。

use windows::Win32::Foundation::{BOOL, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};

/// MONITORINFOF_PRIMARY（windows 0.58 未导出该常量）
const MONITORINFOF_PRIMARY: u32 = 0x0000_0001;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

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

/// 点 (x, y) 落在哪块屏；不在任何屏内时返回最近的一块。
pub fn index_at(list: &[MonitorInfo], x: f32, y: f32) -> usize {
    if list.is_empty() {
        return 0;
    }
    let mut best = 0usize;
    let mut best_d = f32::MAX;
    for (i, m) in list.iter().enumerate() {
        let (ox, oy) = (m.origin[0] as f32, m.origin[1] as f32);
        let (w, h) = (m.size[0] as f32, m.size[1] as f32);
        if x >= ox && x < ox + w && y >= oy && y < oy + h {
            return i;
        }
        let cx = (x - (ox + w * 0.5)).abs();
        let cy = (y - (oy + h * 0.5)).abs();
        let d = cx * cx + cy * cy;
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}
