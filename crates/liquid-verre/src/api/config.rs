//! 上层应用可见的全部配置类型。
//!
//! 约定：
//! - 所有结构体 `Default` + `Serialize`/`Deserialize`（`camelCase`），上层可以直接用 JSON/TOML 喂；
//! - 值类型（`*Config` / `*Params` / `*Options`）**不**标 `#[non_exhaustive]`：
//!   那会让上层没法用 `WindowConfig { rect, ..Default::default() }` 这种写法，
//!   而这恰恰是配置类 API 最重要的可用性。加字段时靠 `Default` + serde `default`
//!   保持源码兼容。枚举（`ErrorKind` / `EngineEvent`）仍然 `#[non_exhaustive]`，
//!   因为那边加变体才是常态。
//! - **任何来自上层的数值都必须先过 `sanitize()` 才进 GPU**（NaN / 负数 / 超大值一律夹紧）。
//! - 长度量纲统一为「逻辑像素 = 主屏物理像素」，跨屏时由库按 DPI 比例换算（方案 12.2.5）。

use serde::{Deserialize, Serialize};

use crate::core::params::LiquidParams;

/// 矩形 / 尺寸：`[x, y, width, height]`，逻辑像素（虚拟屏绝对坐标）。
pub type Rect = [f32; 4];

/// 玻璃渲染模式。
///
/// - [`GlassMode::Liquid`]：完整液态玻璃（折射 + 色散 + 模糊金字塔 + 高光 + 焦散 + 鼠标点光），
///   参数集 = [`LiquidParams`]，约 40 项。
/// - [`GlassMode::Transparent`]：纯透明玻璃（只有圆角 + 透明度 + 颜色），
///   **不采样模糊金字塔**，本屏全是透明块时整条 Pass A 被跳过，成本远低于液态模式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GlassMode {
    /// 液态玻璃（默认）。
    #[default]
    Liquid,
    /// 透明玻璃。
    Transparent,
}

impl GlassMode {
    /// 是否需要桌面采集 + 模糊金字塔（Pass A）。
    #[must_use]
    pub const fn needs_backdrop(self) -> bool {
        matches!(self, GlassMode::Liquid)
    }
}

/// 透明玻璃模式的全部可调参数（需求 F3.2：只有这三项）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TransparentParams {
    /// 圆角半径（逻辑像素）。
    pub corner_radius: f32,
    /// 玻璃透明度 0–1：0 = 完全透明（不可见），1 = 完全不透明。
    pub opacity: f32,
    /// 玻璃颜色 RGBA，各通道 0–1。alpha 通道会与 `opacity` 相乘。
    pub color: [f32; 4],
}

impl Default for TransparentParams {
    fn default() -> Self {
        Self {
            corner_radius: 32.0,
            opacity: 0.35,
            color: [1.0, 1.0, 1.0, 1.0],
        }
    }
}

impl TransparentParams {
    /// 夹紧到合法区间（NaN 归零）。
    pub fn sanitize(&mut self) {
        self.corner_radius = clamp_finite(self.corner_radius, 0.0, 400.0);
        self.opacity = clamp_finite(self.opacity, 0.0, 1.0);
        for c in self.color.iter_mut() {
            *c = clamp_finite(*c, 0.0, 1.0);
        }
    }

    /// 按 DPI 比例缩放像素量纲字段。
    #[must_use]
    pub fn scaled(&self, s: f32) -> Self {
        if (s - 1.0).abs() < 1e-4 {
            return *self;
        }
        let mut p = *self;
        p.corner_radius *= s;
        p.sanitize();
        p
    }
}

/// 玻璃外观 = 模式 + 该模式下的参数集。
///
/// 需求 F3.4：同一个玻璃窗口可以在运行期于两种模式之间切换（`GlassWindow::set_style`）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GlassStyle {
    /// 液态玻璃（全部光学 / 交互参数）。
    Liquid(LiquidParams),
    /// 透明玻璃（圆角 / 透明度 / 颜色）。
    Transparent(TransparentParams),
}

impl Default for GlassStyle {
    fn default() -> Self {
        GlassStyle::Liquid(LiquidParams::default())
    }
}

impl GlassStyle {
    /// 当前模式。
    #[must_use]
    pub fn mode(&self) -> GlassMode {
        match self {
            GlassStyle::Liquid(_) => GlassMode::Liquid,
            GlassStyle::Transparent(_) => GlassMode::Transparent,
        }
    }

    /// 是否需要桌面采集 + 模糊金字塔。
    #[must_use]
    pub fn needs_backdrop(&self) -> bool {
        self.mode().needs_backdrop()
    }

    /// 圆角半径（逻辑像素），两种模式都有。
    #[must_use]
    pub fn corner_radius(&self) -> f32 {
        match self {
            GlassStyle::Liquid(p) => p.corner_radius,
            GlassStyle::Transparent(p) => p.corner_radius,
        }
    }

    /// 阴影外扩边距（逻辑像素）。透明模式恒为 0（没有阴影）。
    ///
    /// 这个值决定窗口区域（`SetWindowRgn`）与逐块绘制矩形的外扩量，
    /// 必须与实际渲染保持一致，否则阴影会被裁掉。
    #[must_use]
    pub fn shadow_margin(&self) -> f32 {
        match self {
            GlassStyle::Liquid(p) => p.shadow_margin.max(0.0),
            GlassStyle::Transparent(_) => 0.0,
        }
    }

    /// 夹紧到合法区间。
    pub fn sanitize(&mut self) {
        match self {
            GlassStyle::Liquid(p) => p.sanitize(),
            GlassStyle::Transparent(p) => p.sanitize(),
        }
    }

    /// 按 DPI 比例缩放像素量纲字段。
    #[must_use]
    pub fn scaled(&self, s: f32) -> GlassStyle {
        match self {
            GlassStyle::Liquid(p) => GlassStyle::Liquid(p.scaled(s)),
            GlassStyle::Transparent(p) => GlassStyle::Transparent(p.scaled(s)),
        }
    }

    /// 展开成渲染用的完整参数表。
    ///
    /// 透明模式映射为一组**惰性**的液态参数：折射 / 模糊 / 高光 / 焦散 / 鼠标光全部关掉，
    /// 只保留 `corner_radius`、`base_alpha = opacity`、`tint_color = color`。
    /// 着色器再按 `mode` 走早退分支（完全不采样纹理），因此这些惰性字段不会进入计算。
    /// 这样做的好处是渲染层只需要一份参数结构，脏区 / 外扩 / DPI 换算逻辑完全复用。
    #[must_use]
    pub fn to_params(&self) -> LiquidParams {
        match self {
            GlassStyle::Liquid(p) => *p,
            GlassStyle::Transparent(t) => {
                let mut p = LiquidParams::inert();
                p.corner_radius = t.corner_radius;
                p.base_alpha = t.opacity * t.color[3];
                p.tint_color = t.color;
                p.tint_opacity = 1.0;
                p.shadow_margin = 0.0;
                p.shadow_opacity = 0.0;
                p
            }
        }
    }

    /// 从完整参数表构造液态样式（便捷入口）。
    #[must_use]
    pub fn liquid(params: LiquidParams) -> GlassStyle {
        GlassStyle::Liquid(params)
    }

    /// 构造透明样式（便捷入口）。
    #[must_use]
    pub fn transparent(corner_radius: f32, opacity: f32, color: [f32; 4]) -> GlassStyle {
        let mut t = TransparentParams {
            corner_radius,
            opacity,
            color,
        };
        t.sanitize();
        GlassStyle::Transparent(t)
    }
}

/// NaN / inf 安全的 clamp。
///
/// `f32::clamp` 遇到 NaN 会**原样返回 NaN**（它只在 `self < lo` / `self > hi`
/// 时替换，NaN 的两个比较都是 false），而 NaN 一旦进 GPU，SDF 会整块消失且
/// 不报任何错 —— 极难排查。因此这里显式处理三种非有限值：
/// NaN → `lo`，`+inf` → `hi`，`-inf` → `lo`。
pub(crate) fn clamp_finite(v: f32, lo: f32, hi: f32) -> f32 {
    if v.is_nan() {
        lo
    } else if v == f32::INFINITY {
        hi
    } else if v == f32::NEG_INFINITY {
        lo
    } else {
        v.clamp(lo, hi)
    }
}

/// 缩放（改变玻璃块大小）交互配置。需求 F2 / F2.3。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ResizeOptions {
    /// 总开关。关掉后退化为「只能拖动位置」（与重构前一致）。
    pub enabled: bool,
    /// 允许边缘拖拽（左 / 右 / 上 / 下）。需求 F2.3。
    pub edges: bool,
    /// 允许对角拖拽（四个圆角）。需求 F2.3。
    pub corners: bool,
    /// 命中带宽度（逻辑像素）。对角命中盒 = 角部 `grip × grip` 的方框。
    pub grip_px: f32,
    /// 缩放后的最小宽 / 高（逻辑像素）。
    pub min_width: f32,
    /// 最小高度（逻辑像素）。
    pub min_height: f32,
    /// 悬停时的白色 border 视觉反馈配置。需求 F2.1 / F2.2。
    pub feedback: ResizeFeedbackOptions,
}

impl Default for ResizeOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            edges: true,
            corners: true,
            // 重构前的右下角 grip 是 22 物理 px；200% DPI 下等于 11 逻辑 px，取 10。
            grip_px: 10.0,
            min_width: 24.0,
            min_height: 24.0,
            feedback: ResizeFeedbackOptions::default(),
        }
    }
}

impl ResizeOptions {
    /// 夹紧到合法区间。
    pub fn sanitize(&mut self) {
        self.grip_px = clamp_finite(self.grip_px, 2.0, 128.0);
        self.min_width = clamp_finite(self.min_width, 8.0, 4096.0);
        self.min_height = clamp_finite(self.min_height, 8.0, 4096.0);
        self.feedback.sanitize();
    }

    /// 边缘拖拽是否实际生效。
    #[must_use]
    pub fn edges_active(&self) -> bool {
        self.enabled && self.edges
    }

    /// 对角拖拽是否实际生效。
    #[must_use]
    pub fn corners_active(&self) -> bool {
        self.enabled && self.corners
    }
}

/// 缩放悬停反馈（加粗白色 border）的外观配置。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ResizeFeedbackOptions {
    /// 是否绘制 border 反馈（关掉后仍可拖拽，只是没有视觉提示）。
    pub enabled: bool,
    /// border 粗细（逻辑像素）。需求要求「加粗」，默认 5（实芯 + 1px AA，见 border_ps.hlsl）。
    pub border_width: f32,
    /// border 不透明度 0–1。
    pub alpha: f32,
    /// border 颜色，默认纯白。
    pub color: [f32; 3],
}

impl Default for ResizeFeedbackOptions {
    fn default() -> Self {
        // 5 逻辑像素 + 全不透明：3px/0.95 在 100% DPI 屏上实测「看得见但不醒目」，
        // 用户反馈触发时不够明显。5 逻辑 px 在 200% DPI 下是 10 物理 px 的纯白实线，
        // 100% DPI 下也有 5 px，两种屏都一眼可辨。
        Self {
            enabled: true,
            border_width: 5.0,
            alpha: 1.0,
            color: [1.0, 1.0, 1.0],
        }
    }
}

impl ResizeFeedbackOptions {
    /// 夹紧到合法区间。
    pub fn sanitize(&mut self) {
        self.border_width = clamp_finite(self.border_width, 0.5, 32.0);
        self.alpha = clamp_finite(self.alpha, 0.0, 1.0);
        for c in self.color.iter_mut() {
            *c = clamp_finite(*c, 0.0, 1.0);
        }
    }
}

/// 窗口层级语义。需求 F4。
///
/// 作用于**画布窗口**（每块显示器一个），因此是「整层玻璃」相对其它应用窗口的层级；
/// 玻璃块之间的叠放次序用 [`ZOrder`] / `GlassWindow::bring_to_front` 等 API 控制。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowLevel {
    /// 一直在最前（`WS_EX_TOPMOST`）。**默认值**，与重构前行为一致。
    #[default]
    AlwaysOnTop,
    /// 普通窗口：随其它窗口正常遮挡，不强制置顶也不强制置底。
    Normal,
    /// 点击到最前：普通窗口 + 点击命中玻璃块时抬到普通窗口层的最上面（不抢焦点）。
    ClickToFront,
    /// 一直被覆盖：置于所有普通窗口之下（仍在桌面之上，否则玻璃不可见）。
    AlwaysOnBottom,
}

impl WindowLevel {
    /// 是否需要 `WS_EX_TOPMOST` 扩展样式。
    #[must_use]
    pub const fn is_topmost(self) -> bool {
        matches!(self, WindowLevel::AlwaysOnTop)
    }

    /// 点击时是否需要抬到普通窗口层最前。
    #[must_use]
    pub const fn raises_on_click(self) -> bool {
        matches!(self, WindowLevel::ClickToFront)
    }
}

/// 玻璃块在画布内的叠放次序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ZOrder {
    /// 抬到最前。
    Front,
    /// 压到最后（一直被同层其它玻璃块覆盖）。
    Back,
    /// 指定绝对序号（越大越靠前；库会在需要时整体归一化，不保证数值长期稳定）。
    Absolute(i32),
    /// 相对当前序号偏移（正数向前，负数向后）。
    Relative(i32),
}

/// 桌面采集配置。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CaptureConfig {
    /// 后端偏好。默认 `Auto`：HDR 输出走 WGC，SDR 输出 DDA 优先、失败回退 WGC。
    pub backend: CaptureBackend,
    /// 采集失效后的退避重建间隔（毫秒）。
    pub retry_backoff_ms: u32,
    /// 没有液态玻璃块的屏的采集保活间隔（毫秒）。
    pub idle_keepalive_ms: u32,
    /// 启动时等待首帧就绪的预算（毫秒，主屏）。
    pub startup_wait_ms: u32,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            backend: CaptureBackend::Auto,
            retry_backoff_ms: 500,
            idle_keepalive_ms: 250,
            startup_wait_ms: 6000,
        }
    }
}

/// 采集后端偏好。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureBackend {
    /// 自动选择（HDR → WGC，SDR → DDA 优先）。
    #[default]
    Auto,
    /// 强制 DXGI Desktop Duplication。
    ForceDda,
    /// 强制 Windows.Graphics.Capture。
    ForceWgc,
}

/// 单个玻璃窗口的创建 / 更新配置。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WindowConfig {
    /// 位置与尺寸（逻辑像素，虚拟屏绝对坐标）。
    pub rect: Rect,
    /// 外观。`None` = 继承引擎默认样式。
    pub style: Option<GlassStyle>,
    /// 缩放交互配置。
    pub resize: ResizeOptions,
    /// 叠放次序。`None` = 追加到最前。
    pub z_order: Option<ZOrder>,
    /// 是否可被鼠标命中（拖动 / 缩放）。关掉后完全点击穿透。
    pub interactive: bool,
    /// 是否可**拖拽移动位置**（需求：两种模式共用的交互参数）。
    ///
    /// `false` 时按住块内部不会开始移动会话，但**缩放把手仍然有效**
    /// （「不能挪位置」和「不能改大小」是两件事，分别由本字段与
    /// [`ResizeOptions::enabled`] 控制）；点击抬层（`WindowLevel::ClickToFront`
    /// 与同层 `bring_to_front`）也仍然生效。
    pub movable: bool,
    /// 是否可见。
    pub visible: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            rect: [64.0, 64.0, 600.0, 400.0],
            style: None,
            resize: ResizeOptions::default(),
            z_order: None,
            interactive: true,
            movable: true,
            visible: true,
        }
    }
}

impl WindowConfig {
    /// 夹紧到合法区间（尺寸下限、NaN）。
    pub fn sanitize(&mut self) {
        for v in self.rect.iter_mut() {
            *v = clamp_finite(*v, -20000.0, 20000.0);
        }
        self.rect[2] =
            self.rect[2].clamp(crate::core::block::MIN_SIZE, crate::core::block::MAX_SIZE);
        self.rect[3] =
            self.rect[3].clamp(crate::core::block::MIN_SIZE, crate::core::block::MAX_SIZE);
        if let Some(s) = self.style.as_mut() {
            s.sanitize();
        }
        self.resize.sanitize();
    }
}

/// 引擎级配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EngineConfig {
    /// 新建窗口的默认外观。
    pub default_style: GlassStyle,
    /// 画布窗口层级。
    pub window_level: WindowLevel,
    /// 新建窗口的默认缩放交互配置。
    pub default_resize: ResizeOptions,
    /// 采集配置。
    pub capture: CaptureConfig,
    /// 自适应质量控制器（方案 12.4.3）。默认开启；level 0 与未启用时逐位一致。
    pub adaptive_quality: bool,
    /// 遮挡剔除：被「z 序在画布之上、且不透明」的窗口**完整覆盖**（判据用含阴影外扩的
    /// 绘制矩形）的玻璃块，不再驱动桌面采集，也不再计入光标影响盒。默认开启。
    ///
    /// 8 Hz 轮询（125 ms），置位需**连续 2 次**命中、解除只需 **1 次** —— 错判方向永远是
    /// 「多渲染」而不是「少渲染」。**不改绘制集合**：块照常进投影与脏区集合，因此
    /// 单帧像素输出逐位不变；唯一可观察的代价是「遮挡移开后 ≤125 ms 内玻璃里是旧桌面」
    /// （恢复帧会取新桌面并整屏重画，不影响正确性）。
    ///
    /// ⚠ 默认层级 [`WindowLevel::AlwaysOnTop`] 下收益**接近 0**：`EnumWindows` 几乎立刻
    /// 撞上自家画布就终止枚举，遮挡者集合恒为空 —— 置顶玻璃本来就不该被普通窗口遮住。
    /// 真正吃到收益的是 `Normal` / `ClickToFront` / `AlwaysOnBottom`。
    /// 「无边框全屏应用压住整屏」由另一条停产门负责（[`fullscreen_hide`]）：那条关的是
    /// **整屏**（隐藏画布上的采集与渲染每一个像素都进不了用户眼睛，属于缺陷修复而不是
    /// 可选优化），与本项的**逐块**粒度不同，两个开关互不替代。
    ///
    /// 取证后门：`GLASS_OCCLUSION=0`（见 `diag/env.rs`），两者任一关闭都禁用。
    pub occlusion_culling: bool,
    /// 前台出现「无边框铺满整块屏」的窗口时，隐藏该屏画布**并整屏停产**
    /// （方案 R-16 + 0.4.2 的停产门 A）。默认开启。
    ///
    /// 目的：别让置顶画布压在游戏上、破坏独占全屏与 MPO（多平面叠加）。命中时该屏的采集与
    /// 渲染全部跳过（画布已 `SW_HIDE`，那些像素进不了用户眼睛），恢复走「采集失效恢复」
    /// 同一条路径（`invalidate()` + `ready = false`）⇒ 零陈旧帧。
    ///
    /// ⚠ **判据只看形状**（`WS_POPUP` + 无 caption/thickframe + 铺满屏 + 是**前台**窗口），
    /// 分不出「游戏」与「无边框最大化的普通应用」（Electron / Tauri / app-mode 浏览器 /
    /// UWP 壳窗口）。0.4.2 起加了 cloaked 否决（`DWMWA_CLOAKED`），排掉「存在但一个像素
    /// 都不画」的那一类（挂起的 UWP 窗口 / 被 DWM 隐藏的窗口），但**可见的**无边框全屏应用
    /// 仍会命中 —— 上层若不想要这个行为，把它关掉。关掉后玻璃会一直画在这些应用之上
    /// （画布默认是 `HWND_TOPMOST`，见 [`WindowLevel`]）。
    ///
    /// 命中 / 退出都会打一行 `[info]`，命中那行**追加**触发窗口的
    /// `class` / `title` / `rect`（身份只在翻转沿构造，4 Hz 轮询路径零分配），
    /// 同时上抛 [`crate::EngineEvent::ExclusiveFullscreen`]。
    ///
    /// 取证后门：`GLASS_FS_HIDE=0`（见 `diag/env.rs`），与本字段取「与」。
    pub fullscreen_hide: bool,
    /// 帧率上限。`None` = 跟随显示器刷新率。
    pub fps_cap: Option<u32>,
    /// 是否把画布限制在前 N 块显示器（`None` = 全部）。
    pub monitor_limit: Option<usize>,
    /// 日志最低级别（仅影响 `log` 的全局过滤器；上层已装 logger 时以对方为准）。
    pub log_level: LogLevel,
    /// 诊断开关。默认全关；等价于原来的 `GLASS_*` 环境变量，
    /// 保留结构化入口是为了让上层应用（而不是环境变量）也能开取证。
    pub diagnostics: DiagnosticsConfig,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            default_style: GlassStyle::default(),
            window_level: WindowLevel::default(),
            default_resize: ResizeOptions::default(),
            capture: CaptureConfig::default(),
            adaptive_quality: true,
            occlusion_culling: true,
            fullscreen_hide: true,
            fps_cap: None,
            monitor_limit: None,
            log_level: LogLevel::Info,
            diagnostics: DiagnosticsConfig::default(),
        }
    }
}

impl EngineConfig {
    /// 夹紧非法值。
    pub fn sanitize(&mut self) {
        self.default_style.sanitize();
        self.default_resize.sanitize();
        self.capture.retry_backoff_ms = self.capture.retry_backoff_ms.clamp(50, 10_000);
        self.capture.idle_keepalive_ms = self.capture.idle_keepalive_ms.clamp(16, 5_000);
        self.capture.startup_wait_ms = self.capture.startup_wait_ms.clamp(0, 60_000);
        if let Some(cap) = self.fps_cap {
            self.fps_cap = Some(cap.clamp(1, 1000));
        }
    }
}

/// 日志级别（避免把 `log::LevelFilter` 直接暴露进公共 API，保持 serde 友好）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    /// 只报错误。
    Error,
    /// 错误 + 警告。
    Warn,
    /// 错误 + 警告 + 信息（默认）。
    #[default]
    Info,
    /// 加上调试信息。
    Debug,
    /// 全量。
    Trace,
}

impl From<LogLevel> for log::LevelFilter {
    fn from(v: LogLevel) -> Self {
        match v {
            LogLevel::Error => log::LevelFilter::Error,
            LogLevel::Warn => log::LevelFilter::Warn,
            LogLevel::Info => log::LevelFilter::Info,
            LogLevel::Debug => log::LevelFilter::Debug,
            LogLevel::Trace => log::LevelFilter::Trace,
        }
    }
}

/// 诊断 / 取证开关（与 `GLASS_*` 环境变量一一对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiagnosticsConfig {
    /// 启动时自建一块居中玻璃。
    ///
    /// 默认 `false`：库不替上层决定要不要有窗口。`tools/` 下的取证脚本与
    /// `playground` 需要一块确定性的默认玻璃，因此把它打开（或等价地设
    /// `GLASS_DEFAULT_BLOCK=1`）。
    pub default_window: bool,
    /// 每 5 秒墙钟窗口打印一行 `[metrics]`（`GLASS_METRICS`；行首字段 `win_s=5.00`）。
    pub metrics: bool,
    // 注意：`GLASS_FORCE_FULL` / `GLASS_VERIFY_VALIDITY` / `GLASS_SKIP_*` 这类**取证开关**
    // 刻意**不**进配置结构体：它们是 tools/ 脚本的排障后门，只在构造期读环境变量
    // （见 `gfx/compositor.rs`）。放进公共配置会让上层误以为这是产品能力，
    // 也会让「配置了却不生效」的旋钮混进 API（重构自检时删掉过两个这样的字段）。
    /// 关闭自适应质量控制器（`GLASS_ADAPTIVE=0`）。
    pub no_adaptive: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::params::{Profile, Shape};

    #[test]
    fn sanitize_is_idempotent_and_clamps_nonsense() {
        let mut c = EngineConfig::default();
        c.sanitize();
        let once = c.clone();
        c.sanitize();
        assert_eq!(
            once, c,
            "sanitize 必须幂等（默认值里的 light_dir 会被归一化，所以不是不动点）"
        );

        let mut bad = EngineConfig {
            fps_cap: Some(1_000_000),
            capture: CaptureConfig {
                retry_backoff_ms: 0,
                idle_keepalive_ms: 1,
                ..Default::default()
            },
            default_resize: ResizeOptions {
                grip_px: f32::NAN,
                ..Default::default()
            },
            ..Default::default()
        };
        bad.sanitize();
        assert_eq!(bad.fps_cap, Some(1000));
        assert!(bad.capture.retry_backoff_ms >= 50);
        assert!(bad.capture.idle_keepalive_ms >= 16);
        assert!(bad.default_resize.grip_px.is_finite());
    }

    #[test]
    fn occlusion_culling_defaults_on_and_survives_partial_json() {
        // 默认必须是「开启」：它不改绘制集合（单帧像素逐位不变），是纯收益项；
        // 关掉它只用于 A/B 取证（AC6 用 GLASS_OCCLUSION=0/1 同稳态帧对比）。
        assert!(EngineConfig::default().occlusion_culling);
        // 容器级 `#[serde(default)]` ⇒ 老配置文件缺这个字段时走 Default，
        // 上层升级库之后不会被静默关掉（与 `movable` 同一类兼容性承诺）。
        let c: EngineConfig = serde_json::from_str("{}").expect("空对象也能反序列化");
        assert!(c.occlusion_culling);
        // sanitize 不许动它：它是布尔开关，没有「非法值」可夹。
        let mut c2 = EngineConfig {
            occlusion_culling: false,
            ..Default::default()
        };
        c2.sanitize();
        assert!(!c2.occlusion_culling, "关掉就是关掉，sanitize 不许翻回来");
    }

    #[test]
    fn fullscreen_hide_defaults_on_and_survives_partial_json() {
        // 默认必须是「开启」：它是重构前就有的产品行为（方案 R-16 —— 别让置顶画布压在
        // 独占全屏游戏上），静默关掉等于让玻璃永远画在游戏之上。
        assert!(EngineConfig::default().fullscreen_hide);
        // 容器级 `#[serde(default)]` ⇒ 老配置文件缺这个字段时走 Default，
        // 上层升级库之后不会被静默关掉（与 `occlusion_culling` / `movable` 同一类兼容性承诺）。
        let c: EngineConfig = serde_json::from_str("{}").expect("空对象也能反序列化");
        assert!(c.fullscreen_hide);
        // camelCase 是跨语言契约（上层可能是 JS / C# / Python）。
        let json = serde_json::to_string(&c).expect("序列化");
        assert!(
            json.contains("\"fullscreenHide\""),
            "字段名必须是 camelCase: {json}"
        );
        // sanitize 不许动它：布尔开关没有「非法值」可夹。
        let mut c2 = EngineConfig {
            fullscreen_hide: false,
            ..Default::default()
        };
        c2.sanitize();
        assert!(!c2.fullscreen_hide, "关掉就是关掉，sanitize 不许翻回来");
    }

    #[test]
    fn transparent_style_exposes_only_three_params() {
        let mut t = TransparentParams {
            corner_radius: 1e9,
            opacity: 5.0,
            color: [2.0, -1.0, 0.5, 9.0],
        };
        t.sanitize();
        assert!(t.corner_radius <= 400.0);
        assert_eq!(t.opacity, 1.0);
        assert_eq!(t.color, [1.0, 0.0, 0.5, 1.0]);
    }

    #[test]
    fn transparent_style_maps_to_inert_params() {
        let s = GlassStyle::transparent(20.0, 0.5, [1.0, 0.0, 0.0, 1.0]);
        let p = s.to_params();
        assert_eq!(s.mode(), GlassMode::Transparent);
        assert!(!s.needs_backdrop(), "透明模式不需要桌面采集");
        assert_eq!(p.corner_radius, 20.0);
        assert!((p.base_alpha - 0.5).abs() < 1e-6);
        assert_eq!(p.tint_color[0], 1.0);
        assert_eq!(p.shadow_margin, 0.0);
        assert_eq!(p.shadow_opacity, 0.0);
        // 折射 / 模糊 / 高光 / 焦散 / 鼠标光必须全部惰性
        assert_eq!(p.refr_strength, 0.0);
        assert_eq!(p.dispersion, 0.0);
        assert_eq!(p.absorption, 0.0);
        assert_eq!(p.caustic_gain, 0.0);
        assert_eq!(p.spec_intensity, 0.0);
        assert_eq!(p.mouse_intensity, 0.0);
        assert_eq!(p.clarity, 1.0);
    }

    #[test]
    fn liquid_style_needs_backdrop() {
        let s = GlassStyle::default();
        assert_eq!(s.mode(), GlassMode::Liquid);
        assert!(s.needs_backdrop());
        assert!(s.shadow_margin() > 0.0);
    }

    #[test]
    fn style_scaled_only_touches_pixel_dims() {
        let s = GlassStyle::transparent(10.0, 0.4, [0.0, 1.0, 0.0, 1.0]);
        let q = s.scaled(2.0);
        assert_eq!(q.corner_radius(), 20.0);
        match q {
            GlassStyle::Transparent(t) => assert!((t.opacity - 0.4).abs() < 1e-6),
            _ => panic!("模式不应因缩放而改变"),
        }
    }

    #[test]
    fn resize_toggles_are_independent() {
        let mut r = ResizeOptions::default();
        assert!(r.edges_active() && r.corners_active());
        r.edges = false;
        assert!(!r.edges_active() && r.corners_active());
        r.enabled = false;
        assert!(
            !r.edges_active() && !r.corners_active(),
            "总开关关掉后两者都失效"
        );
    }

    #[test]
    fn resize_grip_is_clamped() {
        let mut r = ResizeOptions {
            grip_px: f32::NAN,
            min_width: -5.0,
            ..Default::default()
        };
        r.sanitize();
        assert!(r.grip_px.is_finite() && r.grip_px >= 2.0);
        assert!(r.min_width >= 8.0);
    }

    #[test]
    fn window_level_semantics() {
        assert!(WindowLevel::AlwaysOnTop.is_topmost());
        assert!(!WindowLevel::Normal.is_topmost());
        assert!(WindowLevel::ClickToFront.raises_on_click());
        assert!(!WindowLevel::AlwaysOnBottom.raises_on_click());
        assert_eq!(
            WindowLevel::default(),
            WindowLevel::AlwaysOnTop,
            "默认层级必须与重构前一致"
        );
    }

    #[test]
    fn config_roundtrips_through_json() {
        let c = EngineConfig {
            default_style: GlassStyle::transparent(18.0, 0.6, [0.2, 0.4, 0.8, 1.0]),
            window_level: WindowLevel::ClickToFront,
            ..Default::default()
        };
        let json = serde_json::to_string(&c).expect("序列化");
        let back: EngineConfig = serde_json::from_str(&json).expect("反序列化");
        assert_eq!(back, c, "配置必须能 JSON 往返");
        assert!(
            json.contains("\"windowLevel\""),
            "字段名必须是 camelCase: {json}"
        );
    }

    #[test]
    fn movable_defaults_to_true_and_survives_json() {
        // 需求：两种模式共用的「是否可拖拽移动」参数，默认必须可移动，
        // 否则上层升级库之后所有窗口突然拖不动。
        assert!(WindowConfig::default().movable);
        let json = r#"{"rect":[0,0,100,100],"movable":false}"#;
        let c: WindowConfig = serde_json::from_str(json).unwrap();
        assert!(!c.movable);
        let back = serde_json::to_string(&c).unwrap();
        assert!(back.contains("\"movable\":false"), "{back}");
    }

    #[test]
    fn window_config_json_uses_camel_case() {
        let json = r#"{"rect":[0,0,300,200],"interactive":false}"#;
        let c: WindowConfig = serde_json::from_str(json).expect("部分字段也能反序列化");
        assert!(!c.interactive);
        assert_eq!(c.resize, ResizeOptions::default(), "缺省字段走 Default");
    }

    #[test]
    fn clamp_finite_handles_nan() {
        assert_eq!(clamp_finite(f32::NAN, 1.0, 2.0), 1.0);
        assert_eq!(clamp_finite(f32::INFINITY, 1.0, 2.0), 2.0);
        assert_eq!(clamp_finite(f32::NEG_INFINITY, 1.0, 2.0), 1.0);
        assert_eq!(clamp_finite(1.5, 1.0, 2.0), 1.5);
    }

    #[test]
    fn shape_and_profile_are_serializable() {
        // 液态参数里的枚举字段也必须能跨语言边界传输
        let p = LiquidParams {
            shape: Shape::Squircle,
            ..Default::default()
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: LiquidParams = serde_json::from_str(&json).unwrap();
        assert_eq!(back.shape, Shape::Squircle);
        assert_eq!(Profile::default(), Profile::Circular);
    }
}
