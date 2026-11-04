//! 单个显示器的运行期状态：画布窗口 + 交换链 + 采集会话 + 模糊金字塔。
//!
//! 一块屏一套（方案 4.1 / 12.10），分辨率与 DPI 各不相同也各自正确。

use std::time::Instant;

use windows::Win32::Foundation::HWND;

use crate::capture::Capture;
use crate::core::draw::DrawBlock;
use crate::gfx::{Compositor, Device};
use crate::platform::{display::MonitorInfo, window};

/// 一个显示器 = 一个全屏画布窗口 + 一个交换链 + 一个采集会话 + 一套金字塔。
pub(crate) struct MonitorRt {
    pub info: MonitorInfo,
    pub hwnd: HWND,
    pub compositor: Compositor,
    pub capture: Option<Capture>,
    /// 首帧采集是否已就绪（未就绪的屏不显示，避免黑玻璃）。
    pub ready: bool,
    pub region_dirty: bool,
    pub drag_region_full: bool,
    /// 本屏块的**画布局部坐标**投影副本（复用，热路径零分配）。
    pub blocks: Vec<DrawBlock>,
    pub order: Vec<usize>,
    /// 本屏所有块的最大阴影外扩（物理像素）。
    pub margin: f32,
    /// 本屏所有块的最大缩放命中带（物理像素）。
    pub max_grip: f32,
    /// 本屏所有块的最大鼠标光半径（物理像素），用于影响盒。
    pub mouse_radius: f32,
    /// 本屏所有块的最大拖拽滞后补偿（秒），用于 Pass A 采样余量。
    pub max_lag: f32,
    /// 本屏是否存在需要桌面采集的块（液态模式）。全透明屏可整条跳过 Pass A。
    pub needs_backdrop: bool,
    pub new_backdrop: bool,
    pub backdrop_dirty: Option<[i32; 4]>,
    pub copied_px: i64,
    /// 采集失效后的退避重建时间点（方案 4.3）。
    pub retry_at: Instant,
    /// 失败日志只在状态翻转时打一次：锁屏期间每 500 ms 重试一次，
    /// 否则会刷出成千上万行相同告警。
    pub capture_error_logged: bool,
    /// 上次采集时刻：没有玻璃块的屏降到保活频率采集，省掉整条采集开销。
    pub last_capture: Instant,
    /// 本屏当前处于「无块降频采集」状态，用于识别块刚落入本屏的那一帧。
    pub idle_capturing: bool,
    /// 因「独占全屏应用」被我们主动隐藏（方案 R-16）。
    pub hidden_fs: bool,
    /// 因「采集失效」被我们主动隐藏。
    pub hidden_capture: bool,
    /// 因「本屏没有任何玻璃块」被隐藏。
    ///
    /// 实测：在 60 Hz 副屏上挂一个 DComp 画布（哪怕整帧短路、一次 Present 都不发），
    /// 也会把 120 Hz 主屏的合成节拍拖到 60 Hz —— 20 块拖拽 fps 82.0 → 42.3、
    /// p50 单循环 8.41 ms → 16.67 ms。所以空屏必须把画布真正隐藏掉。
    pub hidden_empty: bool,
    /// 本屏变空的时刻（1 秒防抖，避免跨屏拖拽时反复显示/隐藏）。
    pub empty_since: Option<Instant>,
}

impl MonitorRt {
    #[must_use]
    pub fn canvas_origin(&self) -> [f32; 2] {
        [self.info.origin[0] as f32, self.info.origin[1] as f32]
    }

    /// 窗口区域外扩量：阴影边距与缩放命中带取大者。
    ///
    /// 缩放命中带必须**伸出块外**才能从块外抓住边缘（见 `core::handle::hit_handle`），
    /// 因此窗口区域也要外扩同样的量，否则那部分鼠标消息根本到不了我们窗口。
    #[must_use]
    pub fn region_margin(&self) -> f32 {
        self.margin.max(self.max_grip + 2.0)
    }

    /// 光标影响盒半径（画布像素）。
    ///
    /// 必须严格覆盖着色器里所有光标依赖项的支撑半径（不变量 5）：
    /// 鼠标光的 `3*mouse_radius` 与缩放反馈的 `2*grip+16`，取大者，
    /// 再加上阴影外扩（块的绘制矩形本身就比几何矩形大一圈）。
    ///
    /// **单一来源**：`gfx::Compositor` 的脏区推导、`verify_validity` 自检、
    /// 主循环的 `mouse_relevant` 判定都必须用这一个值，否则会出现
    /// 「盒外块的鼠标项不为 0」的拼接缝（历史前科，见 BASELINE 不变量 5）。
    #[must_use]
    pub fn influence_px(&self) -> i32 {
        crate::core::geometry::frame_influence_px(self.mouse_radius, self.max_grip.max(1.0))
            + self.margin.ceil().max(0.0) as i32
    }

    pub fn create(
        gfx: &Device,
        info: MonitorInfo,
        level: crate::api::config::WindowLevel,
        backend_pref: crate::api::config::CaptureBackend,
    ) -> Option<Self> {
        let name = info.name.clone();
        let hwnd = match window::create_canvas(gfx, &info, level) {
            Ok(h) => h,
            Err(e) => {
                log::warn!("{name} 画布窗口创建失败: {e}");
                return None;
            }
        };
        let compositor = match Compositor::new(gfx, hwnd, info.size[0], info.size[1]) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("{name} 合成器初始化失败: {e}");
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
                }
                return None;
            }
        };
        let capture = match Capture::for_monitor_pref(&gfx.device, info.hmonitor, backend_pref) {
            Ok(c) => Some(c),
            Err(e) => {
                // 该屏可能挂在另一块显卡上，或被别的录屏软件占用了 duplication。
                log::warn!("{name} 采集不可用，跳过该屏: {e}");
                None
            }
        };
        let far_past = Instant::now()
            .checked_sub(std::time::Duration::from_secs(3600))
            .unwrap_or_else(Instant::now);
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
            margin: 0.0,
            max_grip: 0.0,
            mouse_radius: 0.0,
            max_lag: 0.0,
            needs_backdrop: false,
            new_backdrop: false,
            backdrop_dirty: None,
            copied_px: 0,
            retry_at: Instant::now(),
            capture_error_logged: false,
            last_capture: far_past,
            idle_capturing: false,
            hidden_fs: false,
            hidden_capture: false,
            hidden_empty: false,
            empty_since: None,
        })
    }

    // 0.5.2 #2：这里**不再有** `show()`。裸 `ShowWindow(SW_SHOWNOACTIVATE)` 会把画布
    // 抬进普通带顶部（首帧就绪那一刻从底部弹到 Chrome/IDE 之上，宿主实测 t+0.6s）。
    // 显示必须与层级同一次 `SetWindowPos` 完成 —— 见 `platform::window::apply_visibility`
    // 与 `show_request`。留着这个方法就是留一个可回退的坑。
}
