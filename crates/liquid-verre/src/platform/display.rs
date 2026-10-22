//! 显示器枚举（方案 4.4 / 12.10）。
//!
//! 全程使用**物理像素**：进程已声明 PER_MONITOR_AWARE_V2，窗口尺寸、采集纹理
//! 尺寸与 UV 映射处在同一坐标系，避免 Windows 位图拉伸导致的模糊。
//! 每块屏幕各自记录 DPI，`scale = dpi / 主屏 dpi` 用于跨屏时保持逻辑尺寸不变。

use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, DEVMODEW, ENUM_CURRENT_SETTINGS,
    HDC, HMONITOR, MONITORINFOEXW,
};
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetShellWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextW, GWL_STYLE, WS_CAPTION, WS_POPUP, WS_THICKFRAME,
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
// 0.4.2（R-E）又往这一层添了两样东西：全仓唯一的 DWM cloaked 查询点
// （`dwm_cloaked`，`platform::occlusion` 也走它）与翻转沿专用的窗口身份读取
// （`window_desc`）—— 同属「桌面层 Win32 查询」，不新开模块。
// ---------------------------------------------------------------------------

/// 查询窗口的 DWM cloaked 状态。`None` = 查询失败（DWM 关闭 / 窗口刚销毁 / 拒绝回答）。
///
/// **返回 `Option<bool>` 而不是 `bool` 是有意的**：两个调用方对「查询失败」的处置方向
/// **相反**，把方向写死在 helper 里就等于替调用方做了错误的决定。
/// - [`super::occlusion`]（遮挡扫描，8 Hz）：`None` ⇒ 当 cloaked ⇒ **不算遮挡者**（宁可多渲染）
/// - [`fullscreen_app_on`]（全屏检测，4 Hz）：`None` ⇒ 当**未** cloaked ⇒ 维持既有判定
///   （不误伤真游戏 —— 隐藏玻璃才是破坏性动作，0.4.2 起它还叠加「整屏停产」）
///
/// 这是全仓**唯一**的 `DwmGetWindowAttribute` 调用点（R-E4 单一来源）。它是跨进程 RPC
/// （走到 DWM 进程，比读窗口字段贵一个量级），因此两个调用方都把它排在全部便宜关卡之后。
#[must_use]
pub(crate) fn dwm_cloaked(hwnd: HWND) -> Option<bool> {
    let mut raw: u32 = 0;
    // SAFETY: 只把栈上 `raw` 的地址与它的真实大小交给 DWM，指针在这次同步调用内有效；
    // `hwnd` 由调用方从 Win32 刚拿到（可能已失效，失效走 Err 分支而不是 UB）。
    match unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut raw as *mut u32 as *mut core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        )
    } {
        Ok(()) => Some(raw != 0),
        Err(_) => None,
    }
}

/// 「形状 + cloaked ⇒ 是否算独占全屏应用」的**纯**判据（单一来源，R-E5）。
///
/// 抽出来是因为真实 HWND 无法在 CI 里构造，而这条判据的每个分支都是 AC14 要覆盖的。
/// `cloaked` 是 `Option<bool>`：`None` = DWM 查询失败（见 [`dwm_cloaked`]）。
/// ⚠ **失败时的保守方向与遮挡扫描相反**：这里 `None` ⇒ 当**未** cloaked ⇒ 维持既有判定。
/// 理由：隐藏玻璃才是破坏性动作，只有在**确知** cloaked 时才否决，这样对真游戏零影响；
/// 写反了就是「DWM 一抽风，玻璃全灭」。
///
/// 三条判据的**顺序即成本顺序**（样式位 → 铺满屏 → cloaked），[`fullscreen_app_on`] 据此
/// 分段付钱。已知取舍（PRD R-E）：判据只看形状，分不出「游戏」与「无边框最大化的普通应用」
/// （Electron / Tauri / app-mode 浏览器 / UWP 壳窗口）—— 上层不想要这个行为就用
/// `EngineConfig::fullscreen_hide = false` 把整条检测关掉。
#[must_use]
pub(crate) fn is_fullscreen_shape(style: u32, covers_monitor: bool, cloaked: Option<bool>) -> bool {
    // 无边框全屏：有 WS_POPUP，没有标题栏、没有可拖拽边框
    if style & WS_POPUP.0 == 0 {
        return false;
    }
    if style & (WS_CAPTION.0 | WS_THICKFRAME.0) != 0 {
        return false;
    }
    if !covers_monitor {
        return false;
    }
    // cloaked 的窗口一个像素都不画（UWP 挂起 / 被 DWM 隐藏），不可能是「独占全屏应用」。
    // `None`（查询失败）**不**否决，理由见上面的保守方向说明。
    cloaked != Some(true)
}

/// 命中「独占全屏应用」时返回**那个前台窗口的 HWND**（供翻转沿打日志），否则 `None`。
///
/// 返回 HWND 而不是 bool 有两个理由（R-E1）：
/// ① 调用方要记录**真正触发判定的那个窗口**，而不是「打日志那一刻恰好在前台的窗口」
///   （两者可能不同，记错了比不记更误导）；
/// ② 4 Hz 轮询路径**零分配** —— 身份字符串只在翻转沿由 [`window_desc`] 构造。
///
/// ⚠ **成本纪律**：[`dwm_cloaked`] 是跨进程 RPC，必须在便宜判据全部通过之后才调用
/// （样式位、`GetWindowRect`、`MonitorFromWindow`、`GetMonitorInfoW`）。绝大多数时候
/// 前台是个普通带标题栏的窗口，第一段就返回 `None` 了，一次 RPC 都不该付。
/// 这与 [`super::occlusion`] 回调的关卡顺序是同一个契约。
///
/// （这段刻意用顿号而不是 `+` 分隔：行首的 `+` 会被 markdown 当成列表项，
/// 于是后面两行变成「懒惰续行」，`clippy::doc_lazy_continuation` 直接报错。）
/// 「这个前台窗口根本不该进入独占全屏判定」的**纯**身份判据。
///
/// 抽成纯函数是为了单测：`HWND` 只是个指针包装，测试里可以随便造几个不同的值把这几条
/// 排除规则钉死；真实窗口没法在 CI 里构造。
///
/// - `fg` 为 null：没有前台窗口（锁屏 / 安全桌面 / 会话切换的瞬间）。
/// - `fg == mon_hwnd`：前台就是我们自己这块屏的画布。
/// - `fg == shell`：**桌面本身**（`Progman`，标题 "Program Manager"）。
///   ⚠ 这条是**实测抓出来的误报源**，不是假想：`Progman` 的样式是 `WS_POPUP` 且无
///   caption/thickframe，rect 铺满整个虚拟屏（本机实测 `[0, 0, 5040, 2080]`，
///   即 3120x2080 与 1920x1080 两块屏合起来），**三条形状判据全中**；
///   而「没有别的窗口持有焦点」时它就是前台窗口 —— 用户点一下壁纸、或者全部最小化，
///   玻璃就会消失并整屏停产。取证时抓到三次，R-E1 那条身份日志的原文是：
///   `检测到独占全屏应用，暂时隐藏玻璃：class=Progman title="Program Manager" rect=[0, 0, 5040, 2080]`。
///   它在 z 序最底、盖不住任何东西，也不可能是「独占全屏应用」。
///   `shell` 为 null 时（`GetShellWindow` 失败）这一条自动失效，退化成修复前的行为，
///   而**不是**把「所有窗口」都当成桌面（那会让 R-16 整个失效）。
///   ⚠ 这条 null 保护是**双保险**：`shell` 为 null 时只有 `fg` 也为 null 才可能相等，
///   而那种情况第一条已经拦下 ⇒ 删掉它单测**不会**报红（本轮实测：等价变异，179 全绿），
///   所以真正守着 AC18 的是第一条与下面那条单测，别把这条当可省项。
///
/// [`is_own_window`]（自家**另一块屏**的画布）不进这个纯函数：它要调 Win32 读类名，
/// 留在调用方，好让本函数保持零 syscall、可单测。
#[must_use]
pub(crate) fn is_non_candidate_foreground(fg: HWND, mon_hwnd: HWND, shell: HWND) -> bool {
    fg.0.is_null() || fg == mon_hwnd || (!shell.0.is_null() && fg == shell)
}

pub(crate) fn fullscreen_app_on(mon_hwnd: HWND) -> Option<HWND> {
    unsafe {
        let fg = GetForegroundWindow();
        // 身份排除放在最前面（一次 GetShellWindow + 几次指针比较，零分配）：
        // 实测误报源 Progman 的**形状判据是全中的**，只能在身份这一层挡掉。
        if is_non_candidate_foreground(fg, mon_hwnd, GetShellWindow()) || is_own_window(fg) {
            return None;
        }
        let style = GetWindowLongPtrW(fg, GWL_STYLE) as u32;
        // 第一段（只读一个窗口字段）：`covers_monitor` 传 `true`、`cloaked` 传 `None`，
        // 于是这次调用**只**评估两条样式判据 —— 判据仍然只有一份（单一来源），
        // 而 4 Hz 轮询在这里就挡掉了绝大多数普通窗口。
        if !is_fullscreen_shape(style, true, None) {
            return None;
        }
        // 第二段（三次内核调用）：窗口矩形必须铺满它所在的那块屏。
        let mut wr = RECT::default();
        if GetWindowRect(fg, &mut wr).is_err() {
            return None;
        }
        let hmon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(hmon, &mut mi).as_bool() {
            return None;
        }
        let m = mi.rcMonitor;
        let covers_monitor =
            wr.left <= m.left && wr.top <= m.top && wr.right >= m.right && wr.bottom >= m.bottom;
        if !covers_monitor {
            return None;
        }
        // 第三段（唯一的跨进程 RPC）：形状全过之后才问 DWM「这窗口是不是被 cloak 了」。
        is_fullscreen_shape(style, covers_monitor, dwm_cloaked(fg)).then_some(fg)
    }
}

/// 窗口的可诊断身份 `(class, title, rect)`。**只在翻转沿调用**（会分配两个 `String`），
/// 绝不能进 4 Hz 轮询或任何每帧路径（R-E1：热路径零堆分配）。
///
/// `GetWindowTextW` 对**跨进程**窗口读的是内核态窗口结构里缓存的标题，**不发 `WM_GETTEXT`**，
/// 因此不会因为对方线程挂起而阻塞（这与「向客户区控件发 `WM_GETTEXT`」是两回事，别混）。
/// 查询失败时对应字段给空串 / 零矩形：身份日志只是排障辅助，不值得为它引入 `Result`。
///
/// ⚠ `title` 为空**不等于**句柄已失效：MSDN 对跨进程窗口写明「没有 caption 时返回空串」，
/// 而能走到本函数的窗口按形状判据必然无 `WS_CAPTION`（本机实测 Progman 读到了
/// "Program Manager"，那是缓存标题恰好存在，不是可依赖的保证）⇒ 排障以 `class` 与 `rect` 为准。
#[must_use]
pub(crate) fn window_desc(hwnd: HWND) -> (String, String, [i32; 4]) {
    let class = read_wide_attr(|buf| {
        // SAFETY: `buf` 是 `read_wide_attr` 栈上的定长数组，按切片传下去，Win32 不会写越界。
        unsafe { GetClassNameW(hwnd, buf) }
    });
    let title = read_wide_attr(|buf| {
        // SAFETY: 同上；`GetWindowTextW` 的容量语义就是「最多写 len-1 个字符 + 结尾 0」。
        unsafe { GetWindowTextW(hwnd, buf) }
    });
    let mut wr = RECT::default();
    let rect = if unsafe { GetWindowRect(hwnd, &mut wr) }.is_ok() {
        [wr.left, wr.top, wr.right, wr.bottom]
    } else {
        [0, 0, 0, 0]
    };
    (class, title, rect)
}

/// 读一个「返回写入字符数」语义的宽字符窗口属性（`GetClassNameW` / `GetWindowTextW` 共用）。
///
/// 两个 API 的返回约定相同：**写入的字符数**（不含结尾 0），失败返回 0。仍然 clamp 一次
/// （[`wide_len`]）—— 与 `platform::window::class_name_is_canvas` 同一条纪律：
/// 「切片不越界」必须是结构性质，不是对 Win32 返回值的信任。
fn read_wide_attr<F: Fn(&mut [u16]) -> i32>(read: F) -> String {
    let mut buf = [0u16; 512];
    let n = read(&mut buf);
    String::from_utf16_lossy(&buf[..wide_len(n, buf.len())])
}

/// Win32「返回写入字符数」类 API 的长度归一化：负数按 0，超长按 `cap` 截断。
///
/// 纯函数、单独测：它是 [`read_wide_attr`] 里那个切片表达式的**唯一**越界面。
///
/// ⚠ 不写 `const fn`：`usize::min` 在本项目锁定的 `rust-version = "1.77"` 下还不是 const
/// （`Ord` 未稳定为 const trait，实测 E0658）。它只在翻转沿被调用，const 化没有收益。
#[must_use]
fn wide_len(n: i32, cap: usize) -> usize {
    if n <= 0 {
        0
    } else {
        (n as usize).min(cap)
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
    use super::{
        index_at_rects, is_fullscreen_shape, is_non_candidate_foreground, wide_len, MonitorInfo,
    };
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{WS_CAPTION, WS_POPUP, WS_THICKFRAME};

    /// 真·无边框全屏的样式位：只有 WS_POPUP，没有 caption / thickframe。
    /// tools/verify_occlusion.py 的 make_fullscreen 探针用的是 WS_POPUP | WS_VISIBLE，
    /// WS_VISIBLE 不参与判据（见 irrelevant_style_bits_do_not_change_the_verdict）。
    const FS_POPUP: u32 = WS_POPUP.0;

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

    // ---- `is_fullscreen_shape`（R-E5 / AC14）----
    // 真实 HWND 在 CI 里造不出来，所以「形状 + cloaked ⇒ 是否算独占全屏」必须是纯函数；
    // 下面每一条都对应一个真实后果，不是凑覆盖率。

    #[test]
    fn borderless_fullscreen_shape_is_recognised() {
        // tools/verify_occlusion.py 的 make_fullscreen 探针就是 WS_POPUP | WS_VISIBLE，
        // 它必须命中（AC16 的 S4/S6 靠这一条），否则 R-16 的正当场景被一起废掉。
        assert!(
            is_fullscreen_shape(FS_POPUP, true, Some(false)),
            "WS_POPUP + 无 caption/thickframe + 铺满屏 + 确知未 cloak ⇒ 命中"
        );
    }

    #[test]
    fn cloaked_window_is_vetoed() {
        assert!(
            !is_fullscreen_shape(FS_POPUP, true, Some(true)),
            "cloaked 窗口一个像素都不画（UWP 挂起 / 被 DWM 隐藏），不可能是独占全屏应用"
        );
    }

    #[test]
    fn failed_cloak_query_keeps_the_legacy_verdict() {
        // ⚠ 本轮最容易写反的一条：保守方向与 platform::occlusion **相反**。
        // 那边 None ⇒ 当 cloaked ⇒ 不算遮挡者（宁可多渲染）；这边 None ⇒ 当**未** cloaked
        // ⇒ 维持既有判定 —— 隐藏玻璃（0.4.2 起还叠加整屏停产）才是破坏性动作，
        // 只有在**确知** cloaked 时才否决，对真游戏零影响。
        // 写反的症状是「DWM 一抽风，玻璃全灭」，而且不会报成我们的错误。
        assert!(
            is_fullscreen_shape(FS_POPUP, true, None),
            "DWM 查询失败 ⇒ 当未 cloaked，维持既有判定"
        );
    }

    #[test]
    fn ordinary_window_shapes_are_never_fullscreen() {
        assert!(
            !is_fullscreen_shape(0, true, Some(false)),
            "非 WS_POPUP：普通应用窗口"
        );
        assert!(
            !is_fullscreen_shape(FS_POPUP, false, Some(false)),
            "没有铺满所在屏（无边框但只占半屏）"
        );
        // make_cover 探针（tools/verify_occlusion.py S1/S2）就是这个形状：它必须**不**命中，
        // 否则遮挡用例会顺手把「整屏停产门 A」也打开，测出来的 dda_acq 就不是 B1 的功劳了。
        assert!(
            !is_fullscreen_shape(FS_POPUP | WS_CAPTION.0, true, Some(false)),
            "带标题栏 ⇒ 不是独占全屏"
        );
        assert!(
            !is_fullscreen_shape(FS_POPUP | WS_THICKFRAME.0, true, Some(false)),
            "带可拖拽边框（无边框但可缩放的窗口）"
        );
        assert!(
            !is_fullscreen_shape(FS_POPUP | WS_CAPTION.0 | WS_THICKFRAME.0, true, Some(false)),
            "两位都在（最常见的普通窗口形状）"
        );
    }

    #[test]
    fn irrelevant_style_bits_do_not_change_the_verdict() {
        // 已知取舍（PRD R-E）：判据只看形状，分不出「游戏」与「无边框最大化的普通应用」。
        // WS_VISIBLE / WS_MAXIMIZE / WS_CLIPSIBLINGS 都不参与判据，因此 Electron / Tauri 的
        // 无边框全屏窗口同样命中 —— 这条测试把「有意如此」钉住；上层不想要这个行为就用
        // EngineConfig::fullscreen_hide = false 关掉整条检测。
        const WS_VISIBLE_BIT: u32 = 0x1000_0000;
        const WS_MAXIMIZE_BIT: u32 = 0x0100_0000;
        const WS_CLIPSIBLINGS_BIT: u32 = 0x0400_0000;
        assert!(is_fullscreen_shape(
            FS_POPUP | WS_VISIBLE_BIT | WS_MAXIMIZE_BIT | WS_CLIPSIBLINGS_BIT,
            true,
            Some(false)
        ));
    }

    #[test]
    fn desktop_progman_is_never_a_fullscreen_app() {
        // 实测误报源：Progman（桌面本身）的形状判据全中，只能靠身份排除挡掉。
        let progman = HWND(0x1234 as *mut core::ffi::c_void);
        let canvas = HWND(0x5678 as *mut core::ffi::c_void);
        assert!(
            is_non_candidate_foreground(progman, canvas, progman),
            "前台是桌面本身时必须直接排除，否则用户点一下壁纸玻璃就消失并停产"
        );
    }

    #[test]
    fn null_foreground_and_own_canvas_are_never_candidates() {
        let canvas = HWND(0x5678 as *mut core::ffi::c_void);
        let shell = HWND(0x1234 as *mut core::ffi::c_void);
        assert!(
            is_non_candidate_foreground(HWND::default(), canvas, shell),
            "没有前台窗口"
        );
        assert!(
            is_non_candidate_foreground(canvas, canvas, shell),
            "前台就是自家画布"
        );
    }

    #[test]
    fn an_unrelated_window_stays_a_candidate() {
        // 反向对照：排除规则不许误伤真正的全屏应用，否则 R-16 就废了（AC16）。
        let game = HWND(0x9abc as *mut core::ffi::c_void);
        let canvas = HWND(0x5678 as *mut core::ffi::c_void);
        let shell = HWND(0x1234 as *mut core::ffi::c_void);
        assert!(!is_non_candidate_foreground(game, canvas, shell));
    }

    #[test]
    fn shell_lookup_failure_degrades_to_the_legacy_behaviour() {
        // GetShellWindow 失败时 shell 为 null：这一条排除必须自动失效。
        let game = HWND(0x9abc as *mut core::ffi::c_void);
        let canvas = HWND(0x5678 as *mut core::ffi::c_void);
        assert!(!is_non_candidate_foreground(game, canvas, HWND::default()));
        assert!(is_non_candidate_foreground(
            HWND::default(),
            canvas,
            HWND::default()
        ));
    }

    #[test]
    fn wide_len_never_yields_an_out_of_range_slice() {
        // window_desc 里那个切片表达式的唯一越界面：Win32 返回负数 / 超长都不许 panic
        // （与 platform::window::class_name_is_canvas 的 clamp 同一条纪律）。
        assert_eq!(wide_len(0, 512), 0);
        assert_eq!(wide_len(-1, 512), 0, "负数按 0 处理");
        assert_eq!(wide_len(i32::MIN, 512), 0);
        assert_eq!(wide_len(7, 512), 7);
        assert_eq!(wide_len(i32::MAX, 512), 512, "超长按容量截断");
    }
}
