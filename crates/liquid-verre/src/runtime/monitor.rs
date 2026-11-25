//! 单个显示器的运行期状态：画布窗口 + 交换链 + 采集会话 + 模糊金字塔。
//!
//! 一块屏一套（方案 4.1 / 12.10），分辨率与 DPI 各不相同也各自正确。

use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;

use crate::api::event::EngineEvent;
use crate::capture::Capture;
use crate::core::draw::DrawBlock;
use crate::error::ErrorKind;
use crate::gfx::{Compositor, Device};
use crate::platform::{display::MonitorInfo, window};

/// 0.5.4 采集黑帧探针的每会话状态机（判据本体在 `crate::capture` 的纯函数，
/// 驱动点在 `runtime::capture_probe_*`，取证见 `tools/verify_black_probe.py`）。
///
/// 背景（宿主实测报告 2026-11，Win10 + Wallpaper Engine）：
/// - 路径 A：WGC 把带 `WDA_EXCLUDEFROMCAPTURE` 的自家全屏画布**整幅涂黑**
///   （`capture/dda.rs` 模块头自证的版本行为），HDR 输出自动切 WGC、
///   DDA 创建失败回退 WGC 都会踩中 ⇒ 玻璃永远是黑底；
/// - 路径 B：MPO 硬件叠加层不进复制帧（WE 视频壁纸一类），壁纸区域为黑。
///
/// 两条路径在采集侧的表现相同（连续全黑帧），**处置必须不同**：
/// A 可以用「摘除排除前后帧内容是否改变」裁决并自动降级 DDA；
/// B 与「纯黑桌面」不可分辨，只许告警，不许隐藏画布（隐藏会把合法用户打成
/// 「分类框消失」，比黑底更糟）。
#[derive(Debug, Clone, Copy)]
pub(crate) enum CapProbe {
    /// 未启用（产品/取证开关、合成采集、SKIP 后门）或本会话已给出结论。
    Done,
    /// 采样期：对「内容变化」的前几帧回读判黑。`black` = 连续全黑计数，
    /// `until` = 采样窗口截止（桌面安静、凑不齐内容帧就放弃，不无限挂）。
    Sampling { black: u8, until: Instant },
    /// 已把画布临时摘出采集排除（`WDA_NONE`），等**一帧**裁决帧：
    /// `h0` = 最后一次全黑采样的网格摘要（`grid_probe` 的 digest），
    /// 裁决帧内容变了 ⇒ 黑是「排除」涂的（路径 A）；没变 ⇒ 桌面本身黑/路径 B。
    /// `until` = 裁决窗口截止（超时必须恢复排除——画布不许滞留可采集态）。
    FlipWait { h0: u64, until: Instant },
}

/// 判定「WGC 自涂黑」所需的连续全黑帧数。自涂黑是**结构性**行为（会话第一帧
/// 就黑），取 2 而不是 1，纯粹是防「桌面还没合成出内容」的瞬态黑帧。
const PROBE_BLACK_NEED: u8 = 2;
/// 采样窗口：8 s 内凑不齐内容帧 ⇒ 桌面太安静，本会话放弃探针（宁缺毋滥）。
const PROBE_SAMPLE_WINDOW: Duration = Duration::from_secs(8);
/// 翻转裁决窗口：affinity 切换本身会让 DWM 重新合成，裁决帧通常 1~2 个 vsync
/// 内到达；1 s 还没等到 ⇒ 恢复排除并放弃（画布摘出排除的时间必须有硬上界）。
const PROBE_FLIP_WINDOW: Duration = Duration::from_secs(1);

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
    /// 黑帧探针总开关（构造期合成一次：产品语义 && `GLASS_BLACK_PROBE!=0`）。
    /// 与 `occlusion_enabled` 同构：运行期只读这个 bool，绝不 `env::var`。
    pub black_probe: bool,
    /// 本会话的黑帧探针状态（见 [`CapProbe`]）。
    pub cap_probe: CapProbe,
    /// 黑帧告警 warn-once：同一屏跨会话也不刷屏（重建循环 500 ms 一次，
    /// 不设这一位就会按退避节奏刷出上百行同样的告警——前科见 R-C3）。
    pub black_warned: bool,
    /// 曾经确认过「WGC 自涂黑」：降级失败后**不再**给新会话武装探针，
    /// 否则退避重建会一遍遍重跑采样+翻转+失败的降级，把日志刷成噪音。
    pub probe_poisoned: bool,
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
        black_probe: bool,
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
            black_probe,
            cap_probe: CapProbe::Done,
            black_warned: false,
            probe_poisoned: false,
        })
        .map(|mut m| {
            // 新会话即新探针：能不能武装完全由 arm_probe 的三态门决定。
            m.arm_probe();
            m
        })
    }

    /// 为**当前采集会话**武装黑帧探针（0.5.4）。
    ///
    /// 调用点必须贴着每一条「`mon.capture = Some(cap)`」的赋值路径
    /// （create / boot 首帧重建 / 退避重建 / 探针自己的降级重建），
    /// 漏贴的后果是新会话永远不判黑。三态门：
    /// - 开关（`black_probe`）关掉 ⇒ Done；
    /// - 本屏曾确认自涂黑 ⇒ 不再武装（防重建循环刷日志）；
    /// - 会话不可探（synth / SKIP 归因后门）⇒ Done。
    pub fn arm_probe(&mut self) {
        self.cap_probe = match &self.capture {
            // 曾确认自涂黑后，**WGC** 会话不再重新武装（黑是必然的，别再翻排除刷日志）；
            // DDA 会话照常武装：降级后它是否健康正是需要观测的对象。
            Some(cap)
                if self.black_probe
                    && !(self.probe_poisoned && cap.backend_name() == "wgc")
                    && cap.probe_armable() =>
            {
                CapProbe::Sampling {
                    black: 0,
                    until: Instant::now() + PROBE_SAMPLE_WINDOW,
                }
            }
            _ => CapProbe::Done,
        };
    }

    /// 是否处于 FlipWait（主循环据此把**裁决帧**从渲染消费路径上摘下来，只喂探针）。
    #[must_use]
    pub fn probe_flipping(&self) -> bool {
        matches!(self.cap_probe, CapProbe::FlipWait { .. })
    }

    /// 墙钟兜底：FlipWait 等不到任何帧（停产门 / 降频 / 采集报错一路 continue）时，
    /// 也必须恢复画布采集排除——「画布可被采到」的危险窗口必须有硬上界。
    /// 每屏每轮路过，非活动态第一条 match 就退出 ⇒ 稳态成本 ≈ 一次枚举比较。
    pub fn probe_deadline_check(&mut self, events: &mut Vec<EngineEvent>) {
        let now = Instant::now();
        match self.cap_probe {
            CapProbe::FlipWait { until, .. } => {
                if now < until {
                    return;
                }
                let ok = window::set_capture_exclusion(self.hwnd, true).is_ok();
                self.cap_probe = CapProbe::Done;
                if let Some(cap) = self.capture.as_mut() {
                    cap.drop_probe_buffers();
                }
                if !self.black_warned {
                    self.black_warned = true;
                    log::warn!(
                        "{} 黑帧探针：裁决帧超时未至，黑帧来源未定（画布排除恢复{}，保持现后端）",
                        self.info.name,
                        if ok {
                            "成功"
                        } else {
                            "失败——系统可能不支持采集亲和性切换"
                        }
                    );
                    events.push(EngineEvent::Warning {
                        kind: ErrorKind::CaptureDegraded,
                        message: format!(
                            "{} 黑帧探针未能完成裁决（画布排除已恢复），保持现后端",
                            self.info.name
                        ),
                    });
                }
            }
            CapProbe::Sampling { until, .. } => {
                if now >= until {
                    self.cap_probe = CapProbe::Done;
                    if let Some(cap) = self.capture.as_mut() {
                        cap.drop_probe_buffers();
                    }
                }
            }
            CapProbe::Done => {}
        }
    }

    /// 对「内容变化」帧判黑并推进状态机。只有会话头 `PROBE_*` 窗口内的 ≤2 帧
    /// 会走到阻塞回读；其余情况第一行 match 退出 ⇒ 稳态热路径零成本。
    ///
    /// `force` = `GLASS_BLACK_PROBE=force`：把「画布仍处于采集排除」的采样帧一律
    /// 视作全黑，供没有 WE / HDR 的机器闭环取证（digest 仍取真实内容，裁决语义不变）。
    pub fn probe_sample_tick(&mut self, gfx: &Device, force: bool, events: &mut Vec<EngineEvent>) {
        let (mut black, until) = match self.cap_probe {
            CapProbe::Sampling { black, until } => (black, until),
            _ => return,
        };
        if Instant::now() >= until {
            self.cap_probe = CapProbe::Done;
            if let Some(cap) = self.capture.as_mut() {
                cap.drop_probe_buffers();
            }
            return;
        }
        let Some(cap) = self.capture.as_mut() else {
            self.cap_probe = CapProbe::Done;
            return;
        };
        let Some((all_black, digest)) = cap.last_frame_black_probe(&gfx.device, &gfx.ctx) else {
            // 没有帧 / 格式不可探：本会话放弃。**不设** poisoned——这不是结论，是没测成。
            self.cap_probe = CapProbe::Done;
            cap.drop_probe_buffers();
            return;
        };
        if !(force || all_black) {
            self.cap_probe = CapProbe::Done;
            cap.drop_probe_buffers();
            return;
        }
        black += 1;
        if black < PROBE_BLACK_NEED {
            self.cap_probe = CapProbe::Sampling { black, until };
            return;
        }
        // 连续全黑。只有 WGC 能裁决「排除涂黑 vs 桌面本来就黑」；DDA 的两条来源
        // （MPO 黑帧 / 纯黑桌面）在采集侧不可分辨 ⇒ 只告警，不隐藏、不换端。
        if cap.backend_name() != "wgc" {
            self.cap_probe = CapProbe::Done;
            cap.drop_probe_buffers();
            if !self.black_warned {
                self.black_warned = true;
                log::warn!(
                    "{} 黑帧探针：DDA 连续 {PROBE_BLACK_NEED} 帧全黑（MPO 叠加层壁纸不进复制帧，或纯黑桌面，采集侧不可分辨）。若桌面确有内容仍显黑，建议壁纸换静态图片 / 关闭 MPO",
                    self.info.name
                );
                events.push(EngineEvent::Warning {
                    kind: ErrorKind::CaptureDegraded,
                    message: format!(
                        "{} DDA 连续全黑帧：疑似 MPO 叠加层未进复制帧（视频壁纸一类），或纯黑桌面",
                        self.info.name
                    ),
                });
            }
            return;
        }
        if window::set_capture_exclusion(self.hwnd, false).is_err() {
            // 翻不动就绝不编造裁决：排除维持原状，本屏收兵（warn-once）。
            self.cap_probe = CapProbe::Done;
            cap.drop_probe_buffers();
            if !self.black_warned {
                self.black_warned = true;
                log::warn!(
                    "{} 黑帧探针：摘除画布采集排除失败，无法裁决黑帧来源（保持现后端）",
                    self.info.name
                );
            }
            return;
        }
        self.cap_probe = CapProbe::FlipWait {
            h0: digest,
            until: Instant::now() + PROBE_FLIP_WINDOW,
        };
    }

    /// 裁决 FlipWait 帧：本帧**只诊断不渲染**（调用方必须绕开正常消费路径）。
    ///
    /// 判据：摘除排除后帧内容变了（digest 不同或非黑）⇒ 黑是「排除」涂的
    /// （WGC 自涂黑，Win10 实测路径 A）；内容逐位没变 ⇒ 桌面本就黑（或 DDA
    /// 也救不了的 MPO），保持现状。`no_downgrade` 由调用方按「后端是否被
    /// 显式强制」计算（force 仿真语境下调用方会放行降级，见 runtime 接线）。
    pub fn probe_adjudicate(
        &mut self,
        gfx: &Device,
        no_downgrade: bool,
        events: &mut Vec<EngineEvent>,
    ) {
        let (h0, until) = match self.cap_probe {
            CapProbe::FlipWait { h0, until } => (h0, until),
            _ => return,
        };
        // 无论裁决结果：画布立刻回到排除态（affinity 只影响**后续**帧，
        // 本帧 owned 纹理的内容不受影响，裁决读的正是它）。
        let restored = window::set_capture_exclusion(self.hwnd, true).is_ok();
        self.cap_probe = CapProbe::Done;
        let timed_out = Instant::now() >= until;
        // 回读放进独立作用域：出块后才能安全替换 self.capture。
        let probe = if timed_out {
            None
        } else {
            match self.capture.as_mut() {
                Some(cap) => {
                    let r = cap.last_frame_black_probe(&gfx.device, &gfx.ctx);
                    cap.drop_probe_buffers();
                    r
                }
                None => None,
            }
        };
        let Some((all_black, digest)) = probe else {
            if !self.black_warned {
                self.black_warned = true;
                log::warn!(
                    "{} 黑帧探针：{}（画布排除{}，保持现后端）",
                    self.info.name,
                    if timed_out {
                        "裁决帧超时未至，黑帧来源未定"
                    } else {
                        "回读不可用（无帧或格式不支持），探针终止"
                    },
                    if restored {
                        "已恢复"
                    } else {
                        "恢复失败——检查系统采集亲和性支持"
                    }
                );
                events.push(EngineEvent::Warning {
                    kind: ErrorKind::CaptureDegraded,
                    message: format!(
                        "{} 黑帧探针未能取得裁决帧，黑帧来源未定（保持现后端）",
                        self.info.name
                    ),
                });
            }
            return;
        };
        let poisoned = digest != h0 || !all_black;
        if !poisoned {
            if !self.black_warned {
                self.black_warned = true;
                log::warn!(
                    "{} 黑帧探针：摘除画布采集排除后内容未变 ⇒ 黑来自桌面本身（纯黑桌面或 MPO 黑帧），非排除涂黑；保持现后端并继续渲染",
                    self.info.name
                );
                events.push(EngineEvent::Warning {
                    kind: ErrorKind::CaptureDegraded,
                    message: format!(
                        "{} 采集连续全黑且非画布排除所致：纯黑桌面或 MPO 黑帧，两者采集侧不可分辨；库保持渲染",
                        self.info.name
                    ),
                });
            }
            return;
        }
        // —— 确认：WGC 把「被排除的自家全屏画布」整幅涂黑（Win10 实测路径 A）——
        self.probe_poisoned = true;
        if no_downgrade {
            if !self.black_warned {
                self.black_warned = true;
                log::warn!(
                    "{} 黑帧探针：确认 WGC 自涂黑；后端系显式强制（GLASS_BACKEND=wgc / ForceWgc），按约定不自动降级——去掉强制后库会自动切 DDA",
                    self.info.name
                );
            }
            events.push(EngineEvent::Warning {
                kind: ErrorKind::CaptureDegraded,
                message: format!(
                    "{} WGC 把被排除的画布涂黑（Win10 行为）；后端被显式强制，库未降级",
                    self.info.name
                ),
            });
            return;
        }
        let name = self.info.name.clone();
        // 降级走专用构造（绕过 GLASS_BACKEND 后门，见 `Capture::downgrade_to_dda`）：
        // 生产语境 env 强制 wgc 时上面 no_downgrade 已拦截，这里只会被取证/自动路径命中。
        match Capture::downgrade_to_dda(&gfx.device, self.info.hmonitor) {
            Ok(cap) => {
                log::warn!(
                    "{} 黑帧探针：WGC 把被排除的自家画布整幅涂黑（Win10 实测），已自动降级 DDA 采集",
                    name
                );
                events.push(EngineEvent::Warning {
                    kind: ErrorKind::CaptureDegraded,
                    message: format!("{name} WGC 自涂黑（排除窗口被画黑），库已自动降级 DDA 后端"),
                });
                self.capture = Some(cap);
                // 与「退避重建成功」同一处置（既有纪律）：金字塔与 present 账本全清。
                self.compositor.invalidate();
                // 降级后的新会话继续武装：真实环境它很快判健康；force 取证里
                // 它负责把「DDA 全黑 ⇒ 只告警不换端」的后半段也走一遍。
                self.arm_probe();
            }
            Err(e) => {
                if !self.black_warned {
                    self.black_warned = true;
                    log::warn!(
                        "{} 黑帧探针：确认 WGC 自涂黑，但 DDA 也不可用（{e}）：建议壁纸换静态图片 / 关闭 MPO / 关闭 HDR 输出",
                        name
                    );
                }
                events.push(EngineEvent::Warning {
                    kind: ErrorKind::CaptureDegraded,
                    message: format!(
                        "{name} WGC 自涂黑且 DDA 不可用（{e}）：建议换静态壁纸 / 关 MPO / 关 HDR"
                    ),
                });
            }
        }
    }

    // 0.5.2 #2：这里**不再有** `show()`。裸 `ShowWindow(SW_SHOWNOACTIVATE)` 会把画布
    // 抬进普通带顶部（首帧就绪那一刻从底部弹到 Chrome/IDE 之上，宿主实测 t+0.6s）。
    // 显示必须与层级同一次 `SetWindowPos` 完成 —— 见 `platform::window::apply_visibility`
    // 与 `show_request`。留着这个方法就是留一个可回退的坑。
}
