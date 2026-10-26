//! 引擎内部状态机 `App`：块管理、跨屏投影、采集等待、逐屏渲染。
//!
//! `App` **只在渲染线程上存在**，因此内部大量使用 `&mut self` 与裸指针回指
//! （`GWLP_USERDATA`），不需要任何锁。上层线程一律通过 `crate::api` 的命令队列交互。

use std::sync::Arc;
use std::time::Instant;

use windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW;
use windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA;

use crate::api::config::{GlassStyle, ResizeOptions, WindowConfig, WindowLevel};
use crate::api::handle::{Command, MonitorSnapshot, Shared, Snapshot, WindowSnapshot};
use crate::api::input::SinkRef;
use crate::capture::Capture;
use crate::core::block::Block;
use crate::core::draw::{DrawBlock, ResizeFeedback};
use crate::core::handle::ResizeEdges;
use crate::core::manager::Manager;
use crate::core::params::LiquidParams;
use crate::core::quality::QualityCtrl;
use crate::core::style::StyleCache;
use crate::gfx::compositor::FrameInput;
use crate::gfx::{Device, GlassRenderer};
use crate::platform::display;
use crate::runtime::monitor::MonitorRt;

/// 一帧的全局输入（各屏共用，坐标为虚拟屏绝对物理像素）。
#[derive(Debug, Clone, Copy)]
pub(crate) struct FrameCtx {
    pub cursor_screen: [f32; 2],
    pub prev_cursor_screen: [f32; 2],
    pub drag_velocity: [f32; 2],
    pub press: f32,
    pub hover_id: Option<u32>,
    pub time: f32,
}

/// 引擎内部状态。单线程持有，不跨线程共享。
pub(crate) struct App {
    pub gfx: Device,
    pub glass: GlassRenderer,
    pub monitors: Vec<MonitorRt>,
    pub manager: Manager,
    pub shared: Arc<Shared>,

    /// 引擎默认样式（`WindowConfig.style = None` 的窗口继承它）。
    pub default_style: GlassStyle,
    /// `default_style` 展开后的参数表，用于影响盒 / 指标等「全局口径」计算。
    pub default_params: LiquidParams,
    /// 画布窗口层级。
    pub level: WindowLevel,
    /// 新建窗口的默认缩放配置。
    pub default_resize: ResizeOptions,

    pub drag: Option<DragSessionRef>,
    /// 当前拖拽会话的最小尺寸（逻辑像素），按下时快照，避免每帧查表。
    pub min_drag_size: [f32; 2],
    pub hover: Option<u32>,
    pub hover_handle: ResizeEdges,
    /// 光标位置：虚拟屏物理像素（绝对坐标，多屏通用）。
    pub cursor: [f32; 2],
    /// `GLASS_CURSOR_AT` 指定的钉死光标位置（诊断用）。
    ///
    /// `Some` 时**整个运行期**都用这个值，不再轮询 `GetCursorPos`：
    /// 逐像素取证必须与「跑测试时鼠标碰巧停在哪」无关，否则同一份二进制
    /// 两次运行会得到不同的图（hover / 缩放把手反馈都是光标驱动的）。
    pub pinned_cursor: Option<[f32; 2]>,
    pub press: f32,
    pub route_state: crate::core::routing::RouteState,
    pub drag_velocity: [f32; 2],

    pub rebuild_monitors: bool,
    /// 上层请求退出（`Command::Shutdown` / ESC）。
    pub shutdown: bool,
    /// 窗口过程置位的「需要重画」标记（z 序变化 / 会话结束）。
    pub geometry_changed: bool,
    pub fs_check_at: Instant,
    /// 逐块遮挡轮询（B1）的上一次时刻。8 Hz 节流，见 `crate::runtime::occlusion`。
    ///
    /// 与 `fs_check_at` 同构但**独立**：全屏检测 4 Hz、遮挡轮询 8 Hz，两者的成本模型与
    /// 延迟承诺都不同，共用一个时间戳会让其中一个的频率被另一个绑架。
    pub occl_check_at: Instant,
    /// `platform::occlusion::opaque_rects_above` 的输出缓冲（跨帧复用）。
    ///
    /// 热路径零堆分配的硬要求：多屏共用这一个 `Vec`，每屏调用前 `clear()`
    /// （被调方**只追加、不清空**，那是它的显式契约）。
    pub occluders: Vec<[i32; 4]>,
    /// 遮挡剔除是否生效。**构造期算一次**：`cfg.occlusion_culling && !env.no_occlusion`。
    ///
    /// 存成 bool 而不是每次查两处，是为了让热路径（8 Hz 轮询与两个消费点）
    /// 绝不 `env::var`（spec：每帧一次进程环境锁 + 一个 `String` 分配）。
    pub occlusion_enabled: bool,
    /// 每屏「全部块被遮挡 ⇒ 采集停产」的状态位（bit i = 第 i 块屏）。
    ///
    /// **只服务日志翻转沿**（R-C3：不许每轮打印），不参与任何判定 —— 真值是
    /// `Block::occluded`。放这里而不是 `MonitorRt`：一个纯日志用途的位不值得让
    /// `runtime/monitor.rs` 新增字段，位掩码也不必跟着 `rebuild_monitors` resize
    /// （那里直接清 0）。超过 32 块屏时高位屏的**日志**静默放弃，判定仍逐块正确。
    pub occl_paused_mask: u32,
    pub t0: Instant,
    pub frames: u64,

    pub quality: QualityCtrl,
    /// 采集后端偏好（`EngineConfig::capture.backend`）。重建采集时沿用。
    pub backend_pref: crate::api::config::CaptureBackend,
    /// 各屏 (origin,size) 缓存，供 `monitor_index_at` 零分配查询。
    pub monitor_rects: Vec<([i32; 2], [i32; 2])>,
    /// 样式展开缓存（避免每帧对每块重跑 `sanitize`）。
    pub style_cache: StyleCache,
    pub block_sinks: Vec<(u32, SinkRef)>,
    /// 每块按压时的焦点目标 HWND（0 = 未设置）。
    pub block_focus_hwnds: Vec<(u32, isize)>,
    /// set_focus_hwnd 防呆守卫的 warn-once 位。
    pub focus_guard_warned: bool,

    pub content_store: crate::gfx::ContentStore,
    /// 命令缓冲（复用，热路径零分配）。
    pub cmd_buf: Vec<Command>,
    /// 快照缓冲（复用）。
    pub snapshot: Snapshot,
    /// 上一帧各块的 (id, 逻辑矩形)，用于判断快照是否真的变了（双缓冲，热路径零分配）。
    pub snapshot_key: Vec<(u32, [i32; 6])>,
    pub snapshot_key_next: Vec<(u32, [i32; 6])>,
    pub snapshot_dirty: bool,
}

/// `App` 里持有的拖拽会话（独立类型别名只是为了签名短一点）。
impl App {
    #[inline]
    pub(crate) fn sink_for(
        &self,
        id: u32,
    ) -> Option<std::sync::Arc<dyn crate::api::input::GlassInputSink>> {
        self.block_sinks
            .iter()
            .find(|(bid, _)| *bid == id)
            .and_then(|(_, sr)| sr.0.clone())
    }

    #[inline]
    pub(crate) fn focus_hwnd_for(&self, id: u32) -> Option<isize> {
        self.block_focus_hwnds
            .iter()
            .find(|(bid, _)| *bid == id)
            .map(|(_, hwnd)| *hwnd)
            .filter(|h| *h != 0)
    }
}

pub(crate) type DragSessionRef = crate::input::DragSession;

impl SpawnSpec {
    /// 从上层传来的 [`WindowConfig`] 构造。
    #[must_use]
    pub(crate) fn new(id: u32, config: WindowConfig) -> Self {
        Self {
            id,
            rect: config.rect,
            style: config.style,
            resize: config.resize,
            z_order: config.z_order,
            interactive: config.interactive,
            movable: config.movable,
            visible: config.visible,
        }
    }

    /// 诊断布局用的简写（继承默认样式 + 默认可交互可见）。
    #[must_use]
    pub(crate) fn diag(id: u32, rect: [f32; 4], style: GlassStyle, resize: ResizeOptions) -> Self {
        Self {
            id,
            rect,
            style: Some(style),
            resize,
            z_order: None,
            interactive: true,
            movable: true,
            visible: true,
        }
    }
}

impl App {
    #[must_use]
    pub fn time(&self) -> f32 {
        self.t0.elapsed().as_secs_f32()
    }

    /// 读取当前光标位置（钉死模式优先）。
    ///
    /// 窗口过程与主循环都必须走这里，不能再各自调 `cursor_pos()` ——
    /// 否则钉死模式下命中测试用真实光标、渲染用钉死光标，两者会打架。
    #[must_use]
    pub fn read_cursor(&self) -> [f32; 2] {
        match self.pinned_cursor {
            Some(c) => c,
            None => {
                let p = crate::input::cursor_pos();
                [p.x as f32, p.y as f32]
            }
        }
    }

    #[must_use]
    pub fn primary_size(&self) -> [f32; 2] {
        match self.monitors.first() {
            Some(m) => [m.info.size[0] as f32, m.info.size[1] as f32],
            None => [1920.0, 1080.0],
        }
    }

    #[must_use]
    pub fn scale_of(&self, monitor: usize) -> f32 {
        self.monitors
            .get(monitor)
            .map_or(1.0, |m| m.info.scale.max(0.05))
    }

    #[must_use]
    pub fn monitor_index_at(&self, x: f32, y: f32) -> usize {
        // 用缓存的 (origin,size) 而不是每帧 clone 一份 Vec<MonitorInfo>（含 String）。
        display::index_at_rects(&self.monitor_rects, x, y)
            .min(self.monitors.len().saturating_sub(1))
    }

    /// 重新计算 `monitor_rects` 缓存（画布创建 / 重建后调用）。
    pub fn refresh_monitor_rects(&mut self) {
        self.monitor_rects.clear();
        self.monitor_rects
            .extend(self.monitors.iter().map(|m| (m.info.origin, m.info.size)));
    }

    /// 把 `App` 指针写进画布窗口的 `GWLP_USERDATA`（窗口过程据此回指）。
    ///
    /// # Safety
    /// `self` 必须在整个画布生命周期内保持地址不变（`App` 在 `run()` 的栈上，
    /// 画布在 `run()` 返回前全部销毁，因此成立）。
    pub unsafe fn bind_hwnds(&mut self) {
        let raw = self as *mut App as isize;
        for mon in self.monitors.iter() {
            SetWindowLongPtrW(mon.hwnd, GWLP_USERDATA, raw);
        }
    }

    /// 解绑全部画布（销毁前必须调用，否则窗口过程会拿到悬垂指针）。
    pub unsafe fn unbind_hwnds(&self) {
        for mon in self.monitors.iter() {
            SetWindowLongPtrW(mon.hwnd, GWLP_USERDATA, 0);
        }
    }
}

/// 把块归位到它中心所在的显示器，并按 DPI 比例换算尺寸（方案 12.2.5 / 12.10 ①）。
pub(crate) fn sync_block_monitor(app: &mut App, id: u32) {
    let Some(b) = app.manager.get(id) else {
        return;
    };
    let (cx, cy) = (b.x + b.width * 0.5, b.y + b.height * 0.5);
    let target = app.monitor_index_at(cx, cy);
    let (old_scale, w, h) = (b.scale, b.width, b.height);
    let new_scale = app.scale_of(target);
    let Some(b) = app.manager.get_mut(id) else {
        return;
    };
    if target != b.monitor || (new_scale - old_scale).abs() > 1e-4 {
        // 逻辑尺寸不变，物理像素尺寸按两屏 DPI 比例换算，观感保持一致。
        let k = new_scale / old_scale.max(1e-3);
        b.width = w * k;
        b.height = h * k;
        b.scale = new_scale;
        b.monitor = target;
    }
    // 钳制：块中心必须留在本屏内（默认不做跨屏大玻璃，方案 12.10 策略 ①）。
    let Some(info) = app.monitors.get(b.monitor).map(|m| m.info.clone()) else {
        return;
    };
    let (ox, oy) = (info.origin[0] as f32, info.origin[1] as f32);
    let (sw, sh) = (info.size[0] as f32, info.size[1] as f32);
    b.x = b.x.clamp(ox - b.width * 0.5, ox + sw - b.width * 0.5);
    b.y = b.y.clamp(oy - b.height * 0.5, oy + sh - b.height * 0.5);
    b.clamp();
}

/// 把块投影到某块屏的画布局部坐标，并展开每块的 DPI 缩放参数。
///
/// 这里同时算出本屏的几个「取最大值」口径：阴影外扩、缩放命中带、鼠标光半径、
/// 拖拽滞后 —— 它们决定窗口区域外扩量与脏区影响盒，必须覆盖本屏所有块。
pub(crate) fn collect_monitor_blocks(app: &mut App, mi: usize) {
    let hover = app.hover;
    let hover_handle = app.hover_handle;
    let default_style = app.default_style;
    let App {
        manager,
        monitors,
        style_cache,
        quality,
        ..
    } = app;
    let Some(mon) = monitors.get_mut(mi) else {
        return;
    };
    let (ox, oy, scale) = (
        mon.info.origin[0] as f32,
        mon.info.origin[1] as f32,
        mon.info.scale.max(0.05),
    );

    mon.blocks.clear();
    mon.order.clear();
    mon.margin = 0.0;
    mon.max_grip = 0.0;
    mon.mouse_radius = 0.0;
    mon.max_lag = 0.0;
    mon.needs_backdrop = false;

    for b in manager.blocks() {
        if b.monitor != mi || !b.visible {
            continue;
        }
        let style = *b.effective_style(&default_style);
        let mut params = style_cache.resolve(b.id, b.style_epoch, scale, &style);
        // 自适应降档（方案 12.4.3）逐块应用：level 0 时是恒等变换，逐位一致。
        quality.apply(&mut params);

        let radius = b.effective_radius(&style);
        let interactive = b.interactive;
        let movable = b.movable;
        let feedback =
            if Some(b.id) == hover && !hover_handle.is_empty() && b.resize.feedback.enabled {
                let fb = b.resize.feedback;
                ResizeFeedback {
                    edges: hover_handle.to_shader_edges(),
                    corners: hover_handle.to_shader_corners(),
                    width: fb.border_width * scale,
                    alpha: fb.alpha,
                    color: fb.color,
                }
            } else {
                ResizeFeedback::NONE
            };

        mon.margin = mon.margin.max(params.shadow_margin.max(0.0));
        if b.resize.enabled {
            mon.max_grip = mon.max_grip.max(b.resize.grip_px.max(0.0) * scale);
        }
        // 透明模式在任何鼠标代码之前就 early-out，输出与光标完全无关；但 inert()
        // 底座里 mouse_radius=20（sanitize 下限），若照单全收，全透明屏上光标在块
        // 附近 92px 内移动就会白白驱动「Clear+Pass B+Present+Commit」而画面逐位不变
        // （独立审查 P2-8，F3.1 性能链路最后一处漏点）。
        // 判据用 mouse_intensity>0 而不是 mode：液态块把强度调 0 时同样不该贡献。
        // 注意 max_grip **不能**一起条件掉：缩放 border 反馈对透明块同样生效。
        if params.mouse_intensity > 0.0 {
            mon.mouse_radius = mon.mouse_radius.max(params.mouse_radius);
        }
        mon.max_lag = mon.max_lag.max(params.lag_seconds);

        mon.blocks.push(DrawBlock {
            id: b.id,
            x: b.x - ox,
            y: b.y - oy,
            width: b.width,
            height: b.height,
            radius,
            z_order: b.z_order,
            scale,
            params,
            mode: style.mode(),
            interactive,
            movable,
            occluded: b.occluded,
            feedback,
        });
    }
    mon.blocks.sort_by_key(|b| b.z_order);
    mon.order.extend(0..mon.blocks.len());
    // 是否需要桌面背景由投影结果汇总（单一来源：DrawBlock::needs_backdrop）。
    mon.needs_backdrop = mon.blocks.iter().any(DrawBlock::needs_backdrop);
}

/// 等待各屏采集首帧就绪。
///
/// 主屏给足 `primary_budget`（必要时重建 duplication），其余屏各给 1.5 秒；
/// 拿不到的屏不显示画布，运行期仍会继续退避重试。
///
/// 只有**需要桌面背景**的屏才等待：全是透明玻璃的屏不依赖采集，
/// 等它反而会白白拖慢启动（需求 F3.1）。
pub(crate) fn wait_for_captures(app: &mut App, primary_budget: std::time::Duration) {
    for mi in 0..app.monitors.len() {
        // 全是透明玻璃的屏不依赖桌面采集，等它只会白白拖慢启动（需求 F3.1）。
        if !app.monitors[mi].needs_backdrop {
            continue;
        }
        let budget = if mi == 0 {
            primary_budget
        } else {
            std::time::Duration::from_millis(1500)
        };
        let deadline = Instant::now() + budget;
        while Instant::now() < deadline {
            let mon = &mut app.monitors[mi];
            if mon.compositor.capture_ready() {
                break;
            }
            let rebuilt = if mon.capture.is_none() {
                match Capture::for_monitor_pref(
                    &app.gfx.device,
                    mon.info.hmonitor,
                    app.backend_pref,
                ) {
                    Ok(cap) => {
                        mon.capture = Some(cap);
                        true
                    }
                    Err(e) => {
                        log::warn!("{} DDA 首帧: {e}", mon.info.name);
                        false
                    }
                }
            } else {
                false
            };
            if rebuilt {
                mon.retry_at = Instant::now() + std::time::Duration::from_millis(250);
            }
            if let Some(cap) = mon.capture.as_mut() {
                match cap.try_capture(&app.gfx.device, &app.gfx.ctx, 32) {
                    Ok(Some((tex, w, h, changed))) => {
                        if mon.compositor.width != w || mon.compositor.height != h {
                            if let Err(e) = mon.compositor.resize(&app.gfx, w, h) {
                                log::warn!("{} 首帧 resize: {e}", mon.info.name);
                            }
                        }
                        // 必须 set_capture：它把采集纹理挂到金字塔的 SRV 上，
                        // `capture_ready()` 才可能变真。漏掉这一步会让启动白等满
                        // 整个 budget（重构时实测：6 s 里一直打「采集首帧超时」，
                        // 第一帧玻璃要等主循环兜底才出现）。
                        if let Err(e) = mon.compositor.set_capture(&app.gfx, tex, changed) {
                            log::warn!("{} 首帧 set_capture: {e}", mon.info.name);
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        log::warn!("{} DDA 首帧: {e}", mon.info.name);
                        mon.capture = None;
                        mon.retry_at = Instant::now() + std::time::Duration::from_millis(250);
                    }
                }
            }
        }
        if !app.monitors[mi].compositor.capture_ready() {
            log::warn!(
                "{} 采集首帧超时，该屏暂不显示玻璃（运行期会继续重试）",
                app.monitors[mi].info.name
            );
        }
    }
}

/// 显示器拓扑变化后重建全部画布（方案 4.1：分辨率 / DPI / 拔插显示器）。
pub(crate) fn rebuild_monitors(app: &mut App) {
    let infos = display::enumerate();
    if infos.is_empty() {
        return;
    }
    // 先建新窗口再销毁旧窗口，避免中间态出现「一块屏都没有」。
    let old: Vec<_> = app.monitors.iter().map(|m| m.hwnd).collect();
    let mut next: Vec<MonitorRt> = Vec::new();
    for info in infos {
        if let Some(mut m) = MonitorRt::create(&app.gfx, info, app.level, app.backend_pref) {
            if let Err(e) = m
                .compositor
                .resize(&app.gfx, m.info.size[0], m.info.size[1])
            {
                log::warn!("{} 金字塔初始化失败: {e}", m.info.name);
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
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
        }
    }
    // 样式缓存里的 DPI 全部作废
    app.style_cache.clear();
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
    // 拔插显示器后屏的下标可能整体错位，旧的「因遮挡停产」状态位不再对应同一块屏。
    // 清 0 的代价只是：下一轮 8 Hz 扫描若仍全部被遮挡，会多打一条「停产」日志。
    app.occl_paused_mask = 0;
    app.snapshot_dirty = true;
    let count = app.monitors.len();
    app.shared
        .emit(crate::api::event::EngineEvent::MonitorsChanged {
            monitor_count: count,
        });
}

/// 渲染一块屏。返回是否真的提交了内容（脏区为空时为 false）。
pub(crate) fn render_monitor(app: &mut App, mi: usize, ctx: &FrameCtx, full_redraw: bool) -> bool {
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
    let input = FrameInput {
        blocks: &mon.blocks,
        order: &mon.order,
        cursor_screen: ctx.cursor_screen,
        canvas_origin: origin,
        prev_cursor_canvas,
        drag_velocity: ctx.drag_velocity,
        time: ctx.time,
        press: ctx.press,
        hover_id: ctx.hover_id,
        full_redraw,
        backdrop_dirty: mon.backdrop_dirty,
        influence_px: mon.influence_px(),
        max_lag_seconds: mon.max_lag,
        needs_backdrop: mon.needs_backdrop,
        content_store: &app.content_store,
    };
    if let Err(e) = mon.compositor.render(gfx, glass, &input) {
        // 直接借用字段：log 宏只在级别开启时求值，克隆一个 String 进每帧渲染路径
        // 是白白给热路径加一次堆分配（spec：热路径零堆分配）。
        log::error!("render({}): {e}", mon.info.name);
        return false;
    }
    mon.compositor.last_dirty_px > 0
}

/// 把当前状态发布到快照，供上层 getter 读取。
///
/// 只在**真的变了**时才写（比对 `snapshot_key`）。
///
/// 调用频率本身已被主循环门到「`snapshot_dirty` 或 4 Hz 兜底」，因此双缓冲 key 省下的
/// 只是「4 Hz 兜底那一跳且状态没变」时的一次 `RwLock::write`（每秒最多 4 次）；
/// 真正避免每帧抢锁的是主循环那道门，不是这里的比对。
pub(crate) fn publish_snapshot(app: &mut App, fps: f32, hz: f32, battery: bool) {
    app.snapshot_key_next.clear();
    app.snapshot.windows.clear();
    for b in app.manager.blocks() {
        let s = b.scale.max(1e-3);
        let style = *b.effective_style(&app.default_style);
        let q = [
            b.x.round() as i32,
            b.y.round() as i32,
            (b.width / s).round() as i32,
            (b.height / s).round() as i32,
            b.z_order,
            // 位掩码必须**覆盖所有会变的布尔位**：少一位就意味着「只有那一位变了」
            // 不会触发快照发布，上层永远读不到翻转（加 `occluded` 时正是这个坑）。
            // 3 位仍远小于 i32 的宽度，将来再加布尔位往后接即可。
            i32::from(b.visible) * 4 + i32::from(b.interactive) * 2 + i32::from(b.occluded),
        ];
        app.snapshot_key_next.push((b.id, q));
        let dragged = app.drag.as_ref().map(|d| d.id()) == Some(b.id);
        let resizing_now = app.drag.as_ref().is_some_and(|d| d.is_resize());
        app.snapshot.windows.push(WindowSnapshot {
            id: b.id,
            rect: [b.x, b.y, b.width / s, b.height / s],
            style,
            mode: style.mode(),
            resize: b.resize,
            z_order: b.z_order,
            inherits_style: b.inherits_style,
            monitor_index: b.monitor,
            scale: s,
            visible: b.visible,
            interactive: b.interactive,
            movable: b.movable,
            hovered: app.hover == Some(b.id),
            moving: dragged && !resizing_now,
            resizing: dragged && resizing_now,
            occluded: b.occluded,
        });
    }
    let monitors_changed = app.snapshot.monitors.len() != app.monitors.len();
    if monitors_changed {
        app.snapshot.monitors = app
            .monitors
            .iter()
            .map(|m| MonitorSnapshot {
                name: m.info.name.clone(),
                origin: m.info.origin,
                size: m.info.size,
                dpi: m.info.dpi,
                scale: m.info.scale,
                primary: m.info.primary,
                capture_backend: m
                    .capture
                    .as_ref()
                    .map_or("none", |c| c.backend_name())
                    .to_string(),
                canvas_visible: m.ready && !m.hidden_fs && !m.hidden_capture && !m.hidden_empty,
                canvas_handle: m.hwnd.0 as isize,
            })
            .collect();
    }
    app.snapshot.default_style = app.default_style;
    app.snapshot.window_level = app.level;
    app.snapshot.default_resize = app.default_resize;
    app.snapshot.quality_level = app.quality.level();
    app.snapshot.fps = fps;
    app.snapshot.refresh_hz = hz;
    app.snapshot.on_battery = battery;

    let changed =
        app.snapshot_dirty || monitors_changed || app.snapshot_key_next != app.snapshot_key;
    if !changed {
        return;
    }
    std::mem::swap(&mut app.snapshot_key, &mut app.snapshot_key_next);
    app.snapshot_dirty = false;
    let shared = Arc::clone(&app.shared);
    let windows = app.snapshot.windows.clone();
    let monitors = app.snapshot.monitors.clone();
    let (style, level, resize, ql, fps, hz, bat) = (
        app.snapshot.default_style,
        app.snapshot.window_level,
        app.snapshot.default_resize,
        app.snapshot.quality_level,
        app.snapshot.fps,
        app.snapshot.refresh_hz,
        app.snapshot.on_battery,
    );
    shared.publish(move |s| {
        s.windows = windows;
        s.monitors = monitors;
        s.default_style = style;
        s.window_level = level;
        s.default_resize = resize;
        s.quality_level = ql;
        s.fps = fps;
        s.refresh_hz = hz;
        s.on_battery = bat;
    });
}

/// [`spawn_block`] 的入参。
///
/// 打包成结构体而不是 8 个位置参数：位置参数里 `interactive` / `visible` 两个
/// 相邻的 `bool` 一旦写反，编译器不会报错，玻璃会「看得见但点不动」。
pub(crate) struct SpawnSpec {
    pub id: u32,
    /// 逻辑像素矩形 `[x, y, w, h]`。
    pub rect: [f32; 4],
    /// `None` = 继承引擎默认样式。
    pub style: Option<GlassStyle>,
    pub resize: ResizeOptions,
    pub z_order: Option<crate::api::config::ZOrder>,
    pub interactive: bool,
    pub movable: bool,
    pub visible: bool,
}

/// 新建一个块（命令层与诊断布局共用）。
pub(crate) fn spawn_block(app: &mut App, spec: SpawnSpec) {
    let SpawnSpec {
        id,
        rect,
        style,
        resize,
        z_order,
        interactive,
        movable,
        visible,
    } = spec;
    let mut b = Block::new(id, rect[0], rect[1], rect[2], rect[3]);
    b.style = style.unwrap_or(app.default_style);
    b.inherits_style = style.is_none();
    b.style_epoch = 1;
    b.resize = resize;
    b.interactive = interactive;
    b.movable = movable;
    b.visible = visible;
    b.z_order = app
        .manager
        .blocks()
        .iter()
        .map(|x| x.z_order)
        .max()
        .unwrap_or(0)
        + 1;
    let id = app.manager.insert(b);
    // Back / Relative 在「先插到最前」之后再套用，否则创建即置底会被默认值吞掉
    // （重构自检发现的 bug：WindowConfig.z_order = Some(Back) 曾静默变成最前）。
    match z_order {
        Some(crate::api::config::ZOrder::Absolute(v)) => {
            if let Some(b) = app.manager.get_mut(id) {
                b.z_order = v;
            }
        }
        Some(o) => {
            app.manager.set_z(id, o);
        }
        None => {}
    }
    sync_block_monitor(app, id);
    for m in app.monitors.iter_mut() {
        m.region_dirty = true;
    }
    app.snapshot_dirty = true;
    app.shared
        .emit(crate::api::event::EngineEvent::WindowCreated { id });
}
