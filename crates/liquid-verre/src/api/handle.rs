//! 上层 ↔ 渲染线程的桥：命令队列 + 状态快照 + [`GlassWindow`] 句柄。
//!
//! 线程模型：库只有一个渲染线程（消息泵 + 采集 + Pass A/B + Present/Commit 全在它上面）。
//! 上层线程**不允许**直接触碰 GPU 资源或 `Manager`，一律通过本模块：
//! - 写：`GlassWindow` 的方法把 [`Command`] 推进队列并置渲染请求位，渲染线程在帧首一次性消费；
//! - 读：渲染线程在状态变化时发布 [`Snapshot`]，`GlassWindow` 的 getter 从快照读，
//!   因此是「最终一致」（延迟 ≤ 1 帧），换来的是读路径**不加锁竞争渲染线程**。
//!
//! 这套设计取代了重构前控制面板的 `&'static SharedParams` 原子量：
//! 面板从此只是本 API 的一个普通使用者（见 `playground/`），不再是库的一部分。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::api::config::{
    EngineConfig, GlassMode, GlassStyle, Rect, ResizeOptions, WindowConfig, WindowLevel, ZOrder,
};
use crate::api::event::{EngineEvent, EventHandler, EVENT_QUEUE_CAP};
use crate::api::input::SinkRef;
use crate::error::Error;

/// 上层下发的命令。渲染线程在帧首一次性消费（`drain_into`）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Command {
    /// 退出主循环。
    Shutdown,
    /// 改引擎默认样式（会刷进所有 `inherits_style` 的块）。
    SetDefaultStyle(GlassStyle),
    /// 改画布窗口层级。
    SetWindowLevel(WindowLevel),
    /// 改新建窗口的默认缩放配置（已存在的窗口不受影响）。
    SetDefaultResize(ResizeOptions),
    /// 新建窗口（id 由调用侧预分配，因此句柄可以立刻返回给上层）。
    Create {
        id: u32,
        config: WindowConfig,
    },
    /// 销毁窗口。
    Destroy {
        id: u32,
    },
    /// 销毁全部窗口。
    DestroyAll,
    /// 设置矩形（逻辑像素）。
    SetRect {
        id: u32,
        rect: Rect,
    },
    /// 只改尺寸（保持左上角）。
    SetSize {
        id: u32,
        width: f32,
        height: f32,
    },
    /// 只改位置。
    SetPosition {
        id: u32,
        x: f32,
        y: f32,
    },
    /// 设置样式；`None` = 恢复继承引擎默认。
    SetStyle {
        id: u32,
        style: Option<GlassStyle>,
    },
    /// 设置缩放交互配置。
    SetResize {
        id: u32,
        options: ResizeOptions,
    },
    /// 只改缩放总开关（`set_resizable` 用，避免读-改-写整份 ResizeOptions）。
    SetResizable {
        id: u32,
        enabled: bool,
    },
    /// 单字段样式补丁，在渲染线程的权威状态上叠加（见 `StylePatch`）。
    PatchStyle {
        id: u32,
        patch: StylePatch,
    },
    /// 设置叠放次序。
    SetZ {
        id: u32,
        order: ZOrder,
    },
    /// 显示 / 隐藏。
    SetVisible {
        id: u32,
        visible: bool,
    },
    /// 是否可被鼠标命中。
    SetInteractive {
        id: u32,
        interactive: bool,
    },
    /// 是否可拖拽移动位置。
    SetMovable {
        id: u32,
        movable: bool,
    },
    SetInputSink {
        id: u32,
        sink: SinkRef,
    },
    /// 更新本块内容纹理（预乘 BGRA8，物理像素，行距 = w_phys * 4）。
    /// dirty 为块内半开区间 [left, top, right, bottom)（物理像素），None = 全量。
    UpdateContent {
        id: u32,
        w_phys: u32,
        h_phys: u32,
        buffer: std::sync::Arc<[u8]>,
        dirty: Option<[u32; 4]>,
    },
    /// 清除本块内容纹理（释放 GPU 资源）。
    ClearContent {
        id: u32,
    },
    /// 登记块按压时的焦点目标 HWND（isize = HWND 形态）。
    ///
    /// hwnd = 0 清除登记。**库不代调 SetFocus**（跨线程焦点没有安全代调方式，
    /// attach 手法实测挂死渲染线程——见 EngineEvent::FocusHint 文档）：
    /// 块被按压且 sink 返回 Consumed 时（含该次按压的抬层），库发出
    /// `EngineEvent::FocusHint { id, hwnd }`，宿主在自己线程里对壳窗口 SetFocus。
    ///
    /// **守卫**：若 hwnd 等于任一画布自身 HWND，登记被丢弃 + warn-once
    /// （画布永不经任何路径获得键盘，WM_KEYDOWN 的 ESC→Shutdown 分支
    /// 因此维持死路径）。
    SetFocusHwnd {
        id: u32,
        /// 目标 HWND（0 = 清除）。
        hwnd: isize,
    },
}

/// 单个玻璃窗口的只读状态（来自快照）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowSnapshot {
    /// 窗口 id。
    pub id: u32,
    /// 矩形（逻辑像素，虚拟屏绝对坐标）。
    pub rect: Rect,
    /// 外观。
    pub style: GlassStyle,
    /// 渲染模式（= `style.mode()`，单独给出便于匹配）。
    pub mode: GlassMode,
    /// 缩放交互配置。
    pub resize: ResizeOptions,
    /// 画布内叠放次序。
    pub z_order: i32,
    /// 是否继承引擎默认样式。
    pub inherits_style: bool,
    /// 所在显示器下标。
    pub monitor_index: usize,
    /// 该屏 DPI 比例。
    pub scale: f32,
    /// 是否可见。
    pub visible: bool,
    /// 是否可交互。
    pub interactive: bool,
    /// 是否可拖拽移动位置。
    pub movable: bool,
    /// 光标是否悬停在本窗口上。
    pub hovered: bool,
    /// 是否正在被拖动。
    pub moving: bool,
    /// 是否正在被缩放。
    pub resizing: bool,
    /// 是否被「z 序在画布之上、且不透明」的窗口**完整覆盖**（8 Hz 轮询，
    /// 置位需连续 2 次命中、解除只需 1 次）。
    ///
    /// 上层据此解释「玻璃为什么不动了」：被遮挡的块既不驱动桌面采集，也不响应光标移动
    /// —— 它的可见像素全被别的窗口挡着，那些工作都是白烧。
    /// **不代表块被隐藏**：绘制集合没有变，遮挡一移开像素就回来（≤125 ms）。
    /// `EngineConfig::occlusion_culling = false`（或 `GLASS_OCCLUSION=0`）时恒为 `false`。
    pub occluded: bool,
}

/// 单个显示器的只读状态。
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorSnapshot {
    /// 显示器名（`\\.\DISPLAY1` 之类）。
    pub name: String,
    /// 物理原点（虚拟屏坐标）。
    pub origin: [i32; 2],
    /// 物理尺寸。
    pub size: [i32; 2],
    /// DPI。
    pub dpi: f32,
    /// 相对主屏的缩放比例。
    pub scale: f32,
    /// 是否主屏。
    pub primary: bool,
    /// 采集后端名（`"dda"` / `"wgc"` / `"synth"` / `"none"`）。
    pub capture_backend: String,
    /// 画布当前是否可见。
    pub canvas_visible: bool,
    /// 画布窗口原生句柄（`HWND` 的 `isize` 形式，0 = 无）。
    pub canvas_handle: isize,
}

/// 引擎整体状态快照（[`Engine::snapshot`](super::Engine::snapshot) 的返回类型）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    /// 全部窗口状态。
    pub windows: Vec<WindowSnapshot>,
    /// 全部显示器状态。
    pub monitors: Vec<MonitorSnapshot>,
    /// 引擎默认样式。
    pub default_style: GlassStyle,
    /// 画布窗口层级。
    pub window_level: WindowLevel,
    /// 新建窗口的默认缩放配置。
    pub default_resize: ResizeOptions,
    /// 自适应质量档位（0 = 不降档）。
    pub quality_level: u8,
    /// 已提交的渲染帧数。
    pub fps: f32,
    /// 主屏刷新率。
    pub refresh_hz: f32,
    /// 是否电池供电。
    pub on_battery: bool,
    /// 快照版本号，每次发布递增。
    pub epoch: u64,
}

/// 单字段样式补丁。
///
/// 为什么需要它：`set_corner_radius` / `set_opacity` / `set_color` 这类 helper 若按
/// 「从快照读当前样式 → 改一个字段 → 整体写回」实现，同一帧内连调两个就会互相覆盖
/// （快照最快也要下一帧才更新）。补丁命令在渲染线程上对 `Manager` 里的权威状态
/// 做读-改-写，天然串行（独立审查 P2-4）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum StylePatch {
    /// 圆角半径（逻辑像素），两种模式都支持。
    CornerRadius(f32),
    /// 不透明度：透明模式的 `opacity` / 液态模式的 `base_alpha`。
    Opacity(f32),
    /// 玻璃颜色：透明模式的 `color` / 液态模式的 `tint_color`。
    TintColor([f32; 4]),
}

/// 引擎与句柄共享的状态。
pub(crate) struct Shared {
    initial_config: EngineConfig,
    next_id: AtomicU32,
    commands: Mutex<VecDeque<Command>>,
    snapshot: RwLock<Snapshot>,
    events: Mutex<VecDeque<EngineEvent>>,
    dropped: AtomicU64,
    handler: RwLock<Option<EventHandler>>,
    render_request: AtomicBool,
    running: AtomicBool,
    /// 渲染线程是否已经启动过（`Engine` 是单次使用的：join 之后不能再 start）。
    started: AtomicBool,
    /// 渲染线程是否已退出。退出后句柄的 setter 一律 no-op，
    /// 否则命令队列会被宿主的后续调用无限撑大（独立审查 P2-5）。
    stopped: AtomicBool,
    fatal: Mutex<Option<Error>>,
    warn_oversize: AtomicBool,
    warn_len: AtomicBool,
}

impl std::fmt::Debug for Shared {
    /// 手写实现：内部是锁与原子量，`derive(Debug)` 会要求每个字段都 `Debug`，
    /// 而 `Mutex<VecDeque<Command>>` 之类打出来对上层没有意义。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("running", &self.running.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

impl Shared {
    pub(crate) fn new(config: EngineConfig) -> Arc<Self> {
        let snap = Snapshot {
            windows: Vec::new(),
            monitors: Vec::new(),
            default_style: config.default_style,
            window_level: config.window_level,
            default_resize: config.default_resize,
            quality_level: 0,
            fps: 0.0,
            refresh_hz: 0.0,
            on_battery: false,
            epoch: 0,
        };
        Arc::new(Self {
            initial_config: config,
            next_id: AtomicU32::new(1),
            commands: Mutex::new(VecDeque::new()),
            snapshot: RwLock::new(snap),
            events: Mutex::new(VecDeque::new()),
            dropped: AtomicU64::new(0),
            handler: RwLock::new(None),
            render_request: AtomicBool::new(false),
            running: AtomicBool::new(false),
            started: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            fatal: Mutex::new(None),
            warn_oversize: AtomicBool::new(false),
            warn_len: AtomicBool::new(false),
        })
    }

    #[must_use]
    pub(crate) fn config(&self) -> &EngineConfig {
        &self.initial_config
    }

    /// 预分配一个窗口 id（调用侧线程完成，因此句柄可以立刻返回）。
    pub(crate) fn alloc_id(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn push(&self, cmd: Command) {
        // 引擎退出后句柄可能还活在宿主手里：静默丢弃，绝不让队列无限增长。
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        if let Ok(mut q) = self.commands.lock() {
            q.push_back(cmd);
        }
        self.request_render();
    }

    /// 标记「渲染线程已退出」（由 `Engine` 的线程闭包调用）。
    pub(crate) fn mark_stopped(&self) {
        self.stopped.store(true, Ordering::Release);
    }

    /// warn-once gate: content size > 8192.
    /// Returns true the first time (caller should log), false thereafter.
    pub(crate) fn warn_content_oversize(&self) -> bool {
        !self.warn_oversize.swap(true, Ordering::Relaxed)
    }

    /// warn-once gate: content buffer length mismatch.
    /// Returns true the first time (caller should log), false thereafter.
    pub(crate) fn warn_content_len(&self) -> bool {
        !self.warn_len.swap(true, Ordering::Relaxed)
    }

    /// 抢占式标记「已启动」；返回 false 表示是第一次。
    pub(crate) fn mark_started(&self) -> bool {
        !self.started.swap(true, Ordering::AcqRel)
    }

    #[must_use]
    pub(crate) fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    /// 渲染线程：把待处理命令搬进复用缓冲（热路径零分配）。
    pub(crate) fn drain_into(&self, out: &mut Vec<Command>) {
        out.clear();
        if let Ok(mut q) = self.commands.lock() {
            out.extend(q.drain(..));
        }
    }

    pub(crate) fn request_render(&self) {
        self.render_request.store(true, Ordering::Release);
    }

    /// 取走渲染请求位（渲染线程每帧调用一次）。
    pub(crate) fn take_render_request(&self) -> bool {
        self.render_request.swap(false, Ordering::Acquire)
    }

    pub(crate) fn emit(&self, event: EngineEvent) {
        if let Ok(h) = self.handler.read() {
            if let Some(f) = h.as_ref() {
                f(&event);
            }
        }
        if let Ok(mut q) = self.events.lock() {
            if q.len() >= EVENT_QUEUE_CAP {
                let overflow = q.len() - EVENT_QUEUE_CAP + 1;
                for _ in 0..overflow {
                    q.pop_front();
                }
                self.dropped.fetch_add(overflow as u64, Ordering::Relaxed);
            }
            q.push_back(event);
        }
    }

    /// 上层轮询事件。队列溢出时会先补一条 `EventsDropped`。
    pub(crate) fn poll_events(&self) -> Vec<EngineEvent> {
        let dropped = self.dropped.swap(0, Ordering::AcqRel);
        let mut out = Vec::new();
        if dropped > 0 {
            out.push(EngineEvent::EventsDropped { count: dropped });
        }
        if let Ok(mut q) = self.events.lock() {
            out.extend(q.drain(..));
        }
        out
    }

    pub(crate) fn set_handler(&self, handler: Option<EventHandler>) {
        if let Ok(mut h) = self.handler.write() {
            *h = handler;
        }
    }

    /// 渲染线程发布快照。`fill` 负责把内容写进复用缓冲，随后整体换入。
    pub(crate) fn publish(&self, build: impl FnOnce(&mut Snapshot)) {
        let Ok(mut snap) = self.snapshot.write() else {
            return;
        };
        build(&mut snap);
        snap.epoch = snap.epoch.wrapping_add(1);
    }

    pub(crate) fn read_snapshot<R>(&self, f: impl FnOnce(&Snapshot) -> R) -> R {
        match self.snapshot.read() {
            Ok(s) => f(&s),
            // 中毒（某个持有者 panic 过）时退化为默认值，不能让上层 API 直接 panic。
            Err(_) => f(&Snapshot::default()),
        }
    }

    pub(crate) fn set_running(&self, v: bool) {
        self.running.store(v, Ordering::Release);
    }

    #[must_use]
    pub(crate) fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    pub(crate) fn set_fatal(&self, e: Error) {
        if let Ok(mut slot) = self.fatal.lock() {
            if slot.is_none() {
                *slot = Some(e);
            }
        }
    }

    pub(crate) fn take_fatal(&self) -> Option<Error> {
        self.fatal.lock().ok().and_then(|mut s| s.take())
    }
}

/// 一个玻璃窗口的句柄。
///
/// `Clone + Send + Sync`：可以在上层任意线程持有与调用。所有 setter 都是**异步**的
/// （推命令 + 请求一帧），延迟 ≤ 1 帧；所有 getter 读的是渲染线程发布的快照，
/// 因此也是最终一致。窗口被销毁后句柄不会失效，只是方法变成 no-op、
/// getter 返回 `None`（用 [`GlassWindow::is_alive`] 判断）。
#[derive(Debug, Clone)]
pub struct GlassWindow {
    id: u32,
    shared: Arc<Shared>,
}

impl PartialEq for GlassWindow {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && Arc::ptr_eq(&self.shared, &other.shared)
    }
}

impl GlassWindow {
    pub(crate) fn new(id: u32, shared: Arc<Shared>) -> Self {
        Self { id, shared }
    }

    /// 窗口 id（进程内唯一，单调递增）。
    #[must_use]
    pub fn id(&self) -> u32 {
        self.id
    }

    /// 窗口是否仍然存在。
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.shared
            .read_snapshot(|s| s.windows.iter().any(|w| w.id == self.id))
    }

    /// 完整状态快照。
    #[must_use]
    pub fn snapshot(&self) -> Option<WindowSnapshot> {
        self.shared
            .read_snapshot(|s| s.windows.iter().find(|w| w.id == self.id).copied())
    }

    /// 矩形（逻辑像素）。
    #[must_use]
    pub fn rect(&self) -> Option<Rect> {
        self.snapshot().map(|w| w.rect)
    }

    /// 尺寸（逻辑像素）。
    #[must_use]
    pub fn size(&self) -> Option<[f32; 2]> {
        self.snapshot().map(|w| [w.rect[2], w.rect[3]])
    }

    /// 位置（逻辑像素，虚拟屏绝对坐标）。
    #[must_use]
    pub fn position(&self) -> Option<[f32; 2]> {
        self.snapshot().map(|w| [w.rect[0], w.rect[1]])
    }

    /// 当前外观。
    #[must_use]
    pub fn style(&self) -> Option<GlassStyle> {
        self.snapshot().map(|w| w.style)
    }

    /// 当前渲染模式。
    #[must_use]
    pub fn mode(&self) -> Option<GlassMode> {
        self.snapshot().map(|w| w.mode)
    }

    /// 所在显示器下标。
    #[must_use]
    pub fn monitor_index(&self) -> Option<usize> {
        self.snapshot().map(|w| w.monitor_index)
    }

    /// 承载本窗口的画布原生句柄（`HWND` 的 `isize` 形式）。
    ///
    /// 上层应用可以用它把自己的 UI 窗口贴在玻璃之上 / 之下（典型场景：玻璃做弹窗底板，
    /// 应用把自绘的 UI 窗口按同样的矩形叠在上面）。
    /// **不要**对它调用 `DestroyWindow` / `SetWindowPos`，层级请用
    /// [`crate::api::Engine::set_window_level`]。
    #[must_use]
    pub fn canvas_handle(&self) -> Option<isize> {
        self.shared.read_snapshot(|s| {
            let mi = s.windows.iter().find(|w| w.id == self.id)?.monitor_index;
            let h = s.monitors.get(mi)?.canvas_handle;
            (h != 0).then_some(h)
        })
    }

    // ---------------- setters ----------------

    /// 设置矩形（逻辑像素）。非法值会被夹紧。
    pub fn set_rect(&self, rect: Rect) {
        let mut cfg_rect = rect;
        for v in cfg_rect.iter_mut() {
            if !v.is_finite() {
                *v = 0.0;
            }
        }
        cfg_rect[2] = cfg_rect[2].clamp(crate::core::block::MIN_SIZE, crate::core::block::MAX_SIZE);
        cfg_rect[3] = cfg_rect[3].clamp(crate::core::block::MIN_SIZE, crate::core::block::MAX_SIZE);
        self.shared.push(Command::SetRect {
            id: self.id,
            rect: cfg_rect,
        });
    }

    /// 只改尺寸（保持左上角）。
    pub fn set_size(&self, width: f32, height: f32) {
        self.shared.push(Command::SetSize {
            id: self.id,
            width,
            height,
        });
    }

    /// 只改位置。
    pub fn set_position(&self, x: f32, y: f32) {
        self.shared.push(Command::SetPosition { id: self.id, x, y });
    }

    /// 设置外观。传 `None` = 恢复继承引擎默认样式。
    pub fn set_style(&self, style: Option<GlassStyle>) {
        self.shared.push(Command::SetStyle { id: self.id, style });
    }

    /// 切到液态玻璃模式（需求 F3.4）。`None` = 沿用当前参数。
    pub fn set_liquid(&self, params: Option<crate::api::LiquidParams>) {
        let base = match params {
            Some(p) => p,
            None => self
                .style()
                .and_then(|s| match s {
                    GlassStyle::Liquid(p) => Some(p),
                    GlassStyle::Transparent(_) => None,
                })
                .unwrap_or_default(),
        };
        self.set_style(Some(GlassStyle::Liquid(base)));
    }

    /// 切到透明玻璃模式（需求 F3.4）。
    pub fn set_transparent(&self, corner_radius: f32, opacity: f32, color: [f32; 4]) {
        self.set_style(Some(GlassStyle::transparent(corner_radius, opacity, color)));
    }

    /// 只改圆角半径（两种模式都支持）。
    ///
    /// 走单字段补丁命令（在渲染线程的权威状态上叠加），因此同一帧内连调
    /// `set_corner_radius` + `set_opacity` 不会互相覆盖（独立审查 P2-4）。
    /// ⚠ 副作用：补丁过任意字段的窗口会被**固化**（`inherits_style = false`），
    /// 之后 `Engine::set_default_style` 不再影响它；要恢复继承请 `set_style(None)`。
    pub fn set_corner_radius(&self, radius: f32) {
        self.shared.push(Command::PatchStyle {
            id: self.id,
            patch: StylePatch::CornerRadius(radius),
        });
    }

    /// 透明模式：只改透明度。液态模式下改的是 `base_alpha`。
    /// 副作用同 [`GlassWindow::set_corner_radius`]。
    pub fn set_opacity(&self, opacity: f32) {
        self.shared.push(Command::PatchStyle {
            id: self.id,
            patch: StylePatch::Opacity(opacity),
        });
    }

    /// 透明模式：只改玻璃颜色。液态模式下改的是 `tint_color`。
    /// 副作用同 [`GlassWindow::set_corner_radius`]。
    pub fn set_color(&self, color: [f32; 4]) {
        self.shared.push(Command::PatchStyle {
            id: self.id,
            patch: StylePatch::TintColor(color),
        });
    }

    /// 设置缩放交互配置（需求 F2.3：可分别开关边缘 / 对角拖拽）。
    pub fn set_resize_options(&self, options: ResizeOptions) {
        let mut o = options;
        o.sanitize();
        self.shared.push(Command::SetResize {
            id: self.id,
            options: o,
        });
    }

    /// 一键开关缩放能力（只翻 `enabled` 一位，其余配置原样保留）。
    pub fn set_resizable(&self, enabled: bool) {
        self.shared.push(Command::SetResizable {
            id: self.id,
            enabled,
        });
    }

    /// Set input routing sink.
    pub fn set_input_sink(
        &self,
        sink: Option<std::sync::Arc<dyn crate::api::input::GlassInputSink>>,
    ) {
        self.shared.push(Command::SetInputSink {
            id: self.id,
            sink: SinkRef(sink),
        });
    }

    /// 登记该块按压时的焦点目标 HWND（宿主壳窗口）。
    ///
    /// hwnd = 0 清除登记。**库不代调 SetFocus**（Win32 无跨线程安全代法，
    /// attach 手法实测会挂死渲染线程，见 `EngineEvent::FocusHint` 文档）：
    /// 块被按压且 sink 返回 Consumed 时，库在**事件队列与 poll_events** 里发出
    /// `EngineEvent::FocusHint { id, hwnd }`，宿主在自己线程收到后对 hwnd 执行
    /// SetFocus（同线程合法必成），或平时直接在 sink 的 Down 回调里 PostMessage
    /// 给自己的壳窗口——两条路等价，焦点始终由宿主自己拥有。
    ///
    /// **守卫**：若 hwnd 等于任一画布自身 HWND，登记被丢弃并打印一次
    /// [warn]——画布永不经任何路径获得键盘。
    ///
    /// isize 即 HWND 形态，与 WindowSnapshot::canvas_handle() 风格一致。
    pub fn set_focus_hwnd(&self, hwnd: isize) {
        self.shared
            .push(Command::SetFocusHwnd { id: self.id, hwnd });
    }

    /// Update content texture (premultiplied BGRA8, physical pixels).
    ///
    /// `buffer` row pitch = `w_phys * 4` (no padding); `dirty` is a half-open
    /// interval `[left, top, right, bottom)` in block-local physical pixels,
    /// `None` = full upload.
    ///
    /// Validation failures (wrong length, size > 8192) are discarded with a
    /// warn-once log; no `Err` is returned. Size 0 is equivalent to
    /// [`clear_content`](Self::clear_content).
    ///
    /// This method does a single `memcpy` (`buffer` -> `Arc<[u8]>`); the actual
    /// texture upload happens on the render thread at frame start, latency <= 1 frame.
    pub fn update_content(&self, w_phys: u32, h_phys: u32, buffer: &[u8], dirty: Option<[u32; 4]>) {
        // 校验决策在 core::content（0-unsafe 纯函数，AC2 单测矩阵盯着判定表）。
        match crate::core::content::plan_update(w_phys, h_phys, buffer.len()) {
            crate::core::content::ContentPlan::Clear => self.clear_content(),
            crate::core::content::ContentPlan::TooLarge => {
                if self.shared.warn_content_oversize() {
                    log::warn!(
                        "[content] update_content id={}: size {}x{} exceeds {}, discarded",
                        self.id,
                        w_phys,
                        h_phys,
                        crate::core::content::MAX_CONTENT_DIM
                    );
                }
            }
            crate::core::content::ContentPlan::LenMismatch => {
                if self.shared.warn_content_len() {
                    log::warn!(
                        "[content] update_content id={}: buffer len {} != w*h*4, discarded",
                        self.id,
                        buffer.len()
                    );
                }
            }
            crate::core::content::ContentPlan::Upload => {
                let copy: std::sync::Arc<[u8]> = buffer.to_vec().into_boxed_slice().into();
                self.shared.push(Command::UpdateContent {
                    id: self.id,
                    w_phys,
                    h_phys,
                    buffer: copy,
                    dirty,
                });
            }
        }
    }

    /// Clear content texture (release GPU memory).
    ///
    /// Call before hiding, recycling, or resizing. Repeated calls or calls on
    /// blocks without content are harmless.
    pub fn clear_content(&self) {
        self.shared.push(Command::ClearContent { id: self.id });
    }

    /// 设置叠放次序（需求 F4）。
    pub fn set_z_order(&self, order: ZOrder) {
        self.shared.push(Command::SetZ { id: self.id, order });
    }

    /// 抬到同层最前。
    pub fn bring_to_front(&self) {
        self.set_z_order(ZOrder::Front);
    }

    /// 压到同层最后（一直被其它玻璃块覆盖）。
    pub fn send_to_back(&self) {
        self.set_z_order(ZOrder::Back);
    }

    /// 显示 / 隐藏。
    pub fn set_visible(&self, visible: bool) {
        self.shared.push(Command::SetVisible {
            id: self.id,
            visible,
        });
    }

    /// 是否可被鼠标命中（关掉后完全点击穿透）。
    pub fn set_interactive(&self, interactive: bool) {
        self.shared.push(Command::SetInteractive {
            id: self.id,
            interactive,
        });
    }

    /// 是否可拖拽移动位置（两种模式共用）。
    ///
    /// `false` 时按住块内部不会开始移动会话；**缩放把手仍然有效**，
    /// 点击抬层也仍然生效（「不能挪位置」≠「不能改大小 / 不能点选」）。
    pub fn set_movable(&self, movable: bool) {
        self.shared.push(Command::SetMovable {
            id: self.id,
            movable,
        });
    }

    /// 销毁本窗口。之后所有 setter 变成 no-op。
    pub fn close(&self) {
        self.shared.push(Command::Destroy { id: self.id });
    }

    /// 引擎是否仍在运行。
    #[must_use]
    pub fn engine_running(&self) -> bool {
        self.shared.is_running()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::{Command, GlassWindow, Shared, StylePatch};
    use crate::api::config::{GlassStyle, ResizeOptions, WindowLevel, ZOrder};
    use crate::api::event::{EngineEvent, EVENT_QUEUE_CAP};

    fn pair() -> (std::sync::Arc<Shared>, GlassWindow) {
        let shared = Shared::new(Default::default());
        let id = shared.alloc_id();
        (shared.clone(), GlassWindow::new(id, shared))
    }

    fn pending(shared: &Shared) -> Vec<Command> {
        let mut out = Vec::new();
        shared.drain_into(&mut out);
        out
    }

    #[test]
    fn ids_are_unique_and_monotone() {
        let shared = Shared::new(Default::default());
        assert_eq!(shared.alloc_id(), 1);
        assert_eq!(shared.alloc_id(), 2);
        assert_eq!(shared.alloc_id(), 3);
    }

    #[test]
    fn setters_enqueue_commands_and_request_render() {
        let (shared, w) = pair();
        assert!(!shared.take_render_request());
        w.set_rect([1.0, 2.0, 300.0, 200.0]);
        assert!(shared.take_render_request(), "写操作必须唤醒渲染线程");
        let cmds = pending(&shared);
        assert_eq!(
            cmds,
            vec![Command::SetRect {
                id: w.id(),
                rect: [1.0, 2.0, 300.0, 200.0]
            }]
        );
    }

    #[test]
    fn set_rect_clamps_nonsense() {
        let (shared, w) = pair();
        w.set_rect([f32::NAN, 0.0, -100.0, 1e12]);
        match pending(&shared).remove(0) {
            Command::SetRect { rect, .. } => {
                assert!(rect.iter().all(|v| v.is_finite()));
                assert!(rect[2] >= crate::core::block::MIN_SIZE);
                assert!(rect[3] <= crate::core::block::MAX_SIZE);
            }
            other => panic!("意外命令 {other:?}"),
        }
    }

    #[test]
    fn mode_switch_helpers_emit_style_commands() {
        let (shared, w) = pair();
        w.set_transparent(12.0, 0.4, [1.0, 0.0, 0.0, 1.0]);
        w.set_liquid(None);
        let cmds = pending(&shared);
        assert_eq!(cmds.len(), 2);
        match &cmds[0] {
            Command::SetStyle {
                style: Some(GlassStyle::Transparent(t)),
                ..
            } => {
                assert!((t.corner_radius - 12.0).abs() < 1e-6);
                assert!((t.opacity - 0.4).abs() < 1e-6);
            }
            other => panic!("意外命令 {other:?}"),
        }
        assert!(matches!(
            cmds[1],
            Command::SetStyle {
                style: Some(GlassStyle::Liquid(_)),
                ..
            }
        ));
    }

    #[test]
    fn movable_toggle_enqueues_its_own_command() {
        let (shared, w) = pair();
        w.set_movable(false);
        w.set_movable(true);
        let cmds = pending(&shared);
        assert_eq!(
            cmds[0],
            Command::SetMovable {
                id: w.id(),
                movable: false
            }
        );
        assert_eq!(
            cmds[1],
            Command::SetMovable {
                id: w.id(),
                movable: true
            }
        );
    }

    #[test]
    fn z_order_helpers() {
        let (shared, w) = pair();
        w.bring_to_front();
        w.send_to_back();
        w.set_z_order(ZOrder::Relative(-3));
        let cmds = pending(&shared);
        assert_eq!(
            cmds[0],
            Command::SetZ {
                id: w.id(),
                order: ZOrder::Front
            }
        );
        assert_eq!(
            cmds[1],
            Command::SetZ {
                id: w.id(),
                order: ZOrder::Back
            }
        );
        assert_eq!(
            cmds[2],
            Command::SetZ {
                id: w.id(),
                order: ZOrder::Relative(-3)
            }
        );
    }

    #[test]
    fn resizable_toggle_only_flips_the_enabled_bit() {
        let (shared, w) = pair();
        shared.publish(|s| {
            s.windows.push(crate::api::handle::WindowSnapshot {
                id: w.id(),
                rect: [0.0, 0.0, 100.0, 100.0],
                style: GlassStyle::default(),
                mode: GlassStyle::default().mode(),
                resize: ResizeOptions {
                    grip_px: 42.0,
                    ..Default::default()
                },
                z_order: 0,
                inherits_style: true,
                monitor_index: 0,
                scale: 1.0,
                visible: true,
                interactive: true,
                movable: true,
                hovered: false,
                moving: false,
                resizing: false,
                occluded: false,
            });
        });
        assert!(w.is_alive());
        assert_eq!(w.rect(), Some([0.0, 0.0, 100.0, 100.0]));
        w.set_resizable(false);
        // 只翻一位：其余字段由渲染线程在权威状态上原地改，
        // 不再走「快照读基线 -> 整体写回」，因此不存在覆盖其余配置的可能
        // （独立审查 P2-4 的修法；保留性由 commands::SetResizable 分支保证）。
        match pending(&shared).remove(0) {
            Command::SetResizable { id, enabled } => {
                assert_eq!(id, w.id());
                assert!(!enabled);
            }
            other => panic!("意外命令 {other:?}"),
        }
    }

    #[test]
    fn getters_return_none_when_window_unknown() {
        let (shared, w) = pair();
        assert!(!w.is_alive());
        assert_eq!(w.rect(), None);
        assert_eq!(w.style(), None);
        // 未知窗口上的 setter 不 panic：命令照常入队，由渲染线程在消费时
        // 发现窗口不存在并发一条 Warning（句柄比窗口活得久是常态，
        // 不能在 setter 侧用快照判活 —— 新建窗口在首帧前快照里也不存在）。
        w.set_corner_radius(10.0);
        w.set_opacity(0.5);
        w.set_color([0.0; 4]);
        let cmds = pending(&shared);
        assert_eq!(cmds.len(), 3);
        assert!(matches!(cmds[0], Command::PatchStyle { .. }));
    }

    #[test]
    fn style_patches_enqueue_in_order_and_never_read_the_snapshot() {
        // P2-4 的回归测试：连调两个单字段 setter 必须产生两条**独立**的补丁命令，
        // 由渲染线程按序叠加；而不是「读快照 -> 改 -> 整体写回」（那样第二条会覆盖第一条）。
        let (shared, w) = pair();
        w.set_corner_radius(20.0);
        w.set_opacity(0.5);
        w.set_color([0.25, 0.5, 0.75, 1.0]);
        let cmds = pending(&shared);
        assert_eq!(cmds.len(), 3);
        assert_eq!(
            cmds[0],
            Command::PatchStyle {
                id: w.id(),
                patch: StylePatch::CornerRadius(20.0)
            }
        );
        assert_eq!(
            cmds[1],
            Command::PatchStyle {
                id: w.id(),
                patch: StylePatch::Opacity(0.5)
            }
        );
        assert_eq!(
            cmds[2],
            Command::PatchStyle {
                id: w.id(),
                patch: StylePatch::TintColor([0.25, 0.5, 0.75, 1.0])
            }
        );
    }

    #[test]
    fn update_content_valid_input_enqueues_upload_command() {
        let (shared, w) = pair();
        // 2x1 BGRA8 = 8 字节，合法；dirty 半开区间原样带进命令（裁剪在 gfx 消费时做）。
        w.update_content(2, 1, &[7u8; 8], Some([0, 0, 1, 1]));
        let cmds = pending(&shared);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            Command::UpdateContent {
                id,
                w_phys,
                h_phys,
                buffer,
                dirty,
            } => {
                assert_eq!(*id, w.id());
                assert_eq!((*w_phys, *h_phys), (2, 1));
                assert_eq!(&buffer[..], &[7u8; 8]);
                assert_eq!(*dirty, Some([0, 0, 1, 1]));
            }
            other => panic!("意外命令 {other:?}"),
        }
    }

    #[test]
    fn update_content_invalid_inputs_are_discarded_without_commands() {
        let (shared, w) = pair();
        // 长度 != w*h*4 => 丢弃
        w.update_content(2, 1, &[0u8; 7], None);
        // 单边超上限 => 丢弃
        w.update_content(8193, 1, &[0u8; 8], None);
        w.update_content(1, 8193, &[0u8; 8], None);
        assert!(
            pending(&shared).is_empty(),
            "非法参数不得入队（可恢复错误契约：丢弃 + warn-once，不是 Err）"
        );
    }

    #[test]
    fn update_content_zero_size_is_equivalent_to_clear() {
        // FR6：w/h=0 不是错误，语义等价 clear_content。
        let (shared, w) = pair();
        w.update_content(0, 5, &[], None);
        let cmds = pending(&shared);
        assert_eq!(cmds.len(), 1, "0 尺寸应转译为恰好一条 ClearContent");
        assert!(matches!(cmds[0], Command::ClearContent { id } if id == w.id()));
    }

    #[test]
    fn clear_content_enqueues_its_command() {
        let (shared, w) = pair();
        w.clear_content();
        let cmds = pending(&shared);
        assert_eq!(cmds.len(), 1);
        assert!(matches!(cmds[0], Command::ClearContent { id } if id == w.id()));
    }

    #[test]
    fn events_are_queued_and_drained_once() {
        let (shared, _w) = pair();
        shared.emit(EngineEvent::Ready { monitor_count: 2 });
        shared.emit(EngineEvent::Shutdown);
        let got = shared.poll_events();
        assert_eq!(got.len(), 2);
        assert!(shared.poll_events().is_empty(), "事件只能被取走一次");
    }

    #[test]
    fn event_queue_drops_oldest_and_reports() {
        let (shared, _w) = pair();
        for i in 0..(EVENT_QUEUE_CAP + 40) {
            shared.emit(EngineEvent::WindowCreated { id: i as u32 });
        }
        let got = shared.poll_events();
        assert!(matches!(got[0], EngineEvent::EventsDropped { count } if count >= 40));
        assert!(got.len() <= EVENT_QUEUE_CAP + 1);
    }

    #[test]
    fn event_handler_is_called() {
        let (shared, _w) = pair();
        let seen = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counter = seen.clone();
        shared.set_handler(Some(std::sync::Arc::new(move |_| {
            counter.fetch_add(1, Ordering::Relaxed);
        })));
        shared.emit(EngineEvent::Ready { monitor_count: 1 });
        assert_eq!(seen.load(Ordering::Relaxed), 1);
        // handler 与队列并存
        assert_eq!(shared.poll_events().len(), 1);
    }

    #[test]
    fn level_default_is_topmost_and_serializable() {
        assert_eq!(WindowLevel::default(), WindowLevel::AlwaysOnTop);
        let json = serde_json::to_string(&WindowLevel::ClickToFront).unwrap();
        assert_eq!(json, "\"clickToFront\"");
    }
}
