//! 液态玻璃的全部可调参数（方案 5.5 / 5.6 / 7.x）。
//!
//! 热更新只改常量缓冲区，不重建纹理。类型对外重导出为
//! [`crate::api::LiquidParams`]；`Copy` 语义（全是标量 / 定长数组），
//! 因此逐块投影复制不产生堆分配（方案 7.3「热路径零堆分配」）。

use serde::{Deserialize, Serialize};

/// 斜面剖面类型（方案 5.4 的 1D LUT 剖面表）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    /// 圆形：sqrt(1-(1-u)^2)，边缘圆滑、放大感强（默认）
    #[default]
    Circular,
    /// 抛物线：u^2，位移线性，性能最好
    Parabolic,
    /// 正弦：sin(u*pi/2)，过渡最柔和
    Sinusoidal,
}

impl Profile {
    /// |dH/du|
    pub fn slope(self, u: f32) -> f32 {
        let u = u.clamp(0.0, 1.0);
        match self {
            Profile::Circular => {
                let x = (1.0 - u).max(1e-4);
                let denom = (1.0 - x * x).max(1e-4).sqrt();
                (x / denom).min(MAX_SLOPE)
            }
            Profile::Parabolic => (2.0 * u).min(MAX_SLOPE),
            Profile::Sinusoidal => (std::f32::consts::FRAC_PI_2
                * (u * std::f32::consts::FRAC_PI_2).cos())
            .min(MAX_SLOPE),
        }
    }
}

pub const MAX_SLOPE: f32 = 3.0;

/// 形状模式
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    /// 圆角矩形（精确欧氏 SDF）
    #[default]
    RoundedRect,
    /// 超椭圆 squircle（曲率连续）
    Squircle,
}

impl Shape {
    /// 返回 (mode, superN)
    pub fn uniform(self) -> (f32, f32) {
        match self {
            Shape::RoundedRect => (0.0, 2.0),
            Shape::Squircle => (1.0, 4.0),
        }
    }
}

/// 所有外观 / 交互参数。上层应用通过 [`crate::api::GlassStyle::Liquid`] 传入。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LiquidParams {
    // ---- 周期 / 几何 ----
    /// 圆角半径（逻辑 px）
    pub corner_radius: f32,
    /// 斜面覆盖：0 = 仅边缘细带，1 = 覆盖整块（lens_coverage）
    pub lens_coverage: f32,
    /// 边缘斜面带宽（逻辑 px）
    pub bevel_width: f32,
    /// 形状模式
    pub shape: Shape,
    /// 面板超椭圆指数（`shape = Squircle` 时生效）
    pub super_n: f32,
    /// 斜面剖面（折射 LUT 的形状），默认圆形
    pub profile: Profile,

    // ---- 折射 ----
    /// 玻璃折射率 IOR（1.0–2.0，默认 1.5）
    pub ior: f32,
    /// 等效厚度（逻辑 px）
    pub thickness: f32,
    /// 折射强度 0–2
    pub refr_strength: f32,
    /// RGB 色散 0–1
    pub dispersion: f32,
    /// Beer-Lambert 吸收强度 0–1
    pub absorption: f32,

    // ---- 毛玻璃 ↔ 全透 ----
    /// 清晰度 0–1：0 毛玻璃，1 全透折射
    pub clarity: f32,
    /// 最大模糊半径（逻辑 px）
    pub blur_max: f32,
    /// 最小模糊半径（逻辑 px）
    pub blur_min: f32,
    /// 渐进曲线指数
    pub progressive_curve: f32,
    /// 边缘始终保留的清晰折射（默认 0.85）
    pub edge_always: f32,

    // ---- 染色 ----
    /// 染色不透明度 0–1
    pub tint_opacity: f32,
    /// 自适应染色（按环境亮度）0–1
    pub adaptive_tint: f32,
    /// 基础 alpha
    pub base_alpha: f32,
    /// 染色颜色（RGBA，0–1）
    pub tint_color: [f32; 4],
    /// 环境色混入比例
    pub env_tint_mix: f32,

    // ---- 光照 ----
    /// 主光方向（指向光）
    pub light_dir: [f32; 3],
    /// 主光强度
    pub light_intensity: f32,
    /// 高光强度
    pub spec_intensity: f32,
    /// 高光指数
    pub spec_power: f32,
    /// 高光扩散 0–1
    pub spec_spread: f32,
    /// Fresnel F0
    pub fresnel_f0: f32,
    /// 描边宽度（逻辑 px）
    pub rim_width: f32,
    /// 焦散增益
    pub caustic_gain: f32,

    // ---- 鼠标点光 ----
    /// 鼠标半径（逻辑 px）
    pub mouse_radius: f32,
    /// 鼠标强度
    pub mouse_intensity: f32,
    /// 鼠标高光扩散
    pub mouse_spread: f32,
    /// 鼠标高光颜色
    pub mouse_color: [f32; 3],
    /// 邻近余晖
    pub proximity_glow: f32,

    // ---- 其他 ----
    /// 阴影不透明度
    pub shadow_opacity: f32,
    /// 阴影边距（逻辑 px）
    pub shadow_margin: f32,
    /// 拖拽滞后补偿秒数
    pub lag_seconds: f32,
}

impl Default for LiquidParams {
    fn default() -> Self {
        Self {
            // 方案 v1.1 推荐默认值：Apple 风格液态玻璃
            // 对齐 liquid-glass-rs FrostedLiquid 预设，同时保留附件中的物理折射参数。
            corner_radius: 32.0,
            lens_coverage: 0.15,
            bevel_width: 26.0,
            shape: Shape::RoundedRect,
            super_n: 4.0,
            profile: Profile::Circular,

            ior: 1.50,
            thickness: 26.0,
            refr_strength: 1.0,
            dispersion: 0.25,
            absorption: 0.18,

            clarity: 0.10,
            blur_max: 20.0,
            // ⚠ blur_min = 2.4（不是设计值 3.0）、下面 tint_opacity = 0.01（不是 0.018）。
            // 这两个"看起来像手误"的数字是**重构前那个二进制稳态运行时真正渲染出来的值**，
            // 逐像素取证要求与它 0 差异，因此必须原样保留。来历（实测四路对照得出）：
            //
            //   重构前 `src/ui/panel.rs` 的 `SharedParams::apply()` 在主循环里**每帧**把
            //   参数写回 app.params，而 SharedParams 用 `Arc<AtomicIsize>` 存值：
            //     tint_opacity: (0.018*100.0) as isize = 1（截断）-> 读回 0.01
            //     blur_min:     面板没有滑杆，apply() 里按 blur_max*0.12 派生 -> 20*0.12 = 2.4
            //   其余 16 个面板字段的量化往返经逐个核对都恰好无损，只有这两个变了。
            //
            //   于是旧二进制存在**两帧不一致**：引导帧（面板 apply 之前）用 0.018/3.0，
            //   稳态帧（之后，占运行时间的 99.99%+）用 0.01/2.4。
            //   tools/cmp_visual.py 实测：旧二进制自己这两帧相差 873748 像素、最大通道差 140。
            //
            //   "当前视觉效果"只能以稳态帧为准（用户实际长期看到的就是它），所以库的默认值
            //   对齐 0.01/2.4。库化之后面板被移出（见 playground/），不再有量化往返，
            //   引导帧与稳态帧从此**同一份参数**——顺带修掉了旧实现的两帧不一致。
            //   改这两个值 = 改视觉基线，必须同时更新 BASELINE.md 与取证图。
            // ⚠ 必须写成表达式 `20.0 * 0.12` 而**不是**字面量 2.4：
            // f32 下 20.0*0.12 = 2.3999999（0x40199999），而字面量 2.4 = 2.4000001
            // （0x4019999A），差 1 ULP。旧面板就是按 blur_max*0.12 现算的，
            // 用字面量会让 cmp_visual 的 single 场景多出 ~2400 个 ±1 LSB 差异像素
            // （实测：interior 687 + ring 1679 + outside 4，全部最大通道差 = 1）。
            blur_min: 20.0 * 0.12,
            progressive_curve: 1.6,
            edge_always: 0.55,

            // Web 参考实现没有黑色面板底色；仅保留极轻的乳白 tint。
            tint_opacity: 0.01,
            adaptive_tint: 0.35,
            base_alpha: 1.0,
            tint_color: [0.92, 0.95, 1.0, 1.0],
            env_tint_mix: 0.25,

            light_dir: [-0.45, -0.72, 0.53],
            light_intensity: 1.0,
            spec_intensity: 0.85,
            spec_power: 42.0,
            spec_spread: 0.20,
            fresnel_f0: 0.14,
            rim_width: 1.5,
            caustic_gain: 0.55,

            mouse_radius: 220.0,
            mouse_intensity: 1.35,
            mouse_spread: 160.0,
            mouse_color: [1.0, 1.0, 1.0],
            proximity_glow: 0.35,

            shadow_opacity: 0.10,
            shadow_margin: 24.0,
            lag_seconds: 0.012,
        }
    }
}

// 光标影响盒半径已迁到 `crate::core::geometry`（与缩放反馈影响盒共用单一来源）。

impl LiquidParams {
    /// 按显示器 DPI 比例缩放所有「像素量纲」的参数（方案 12.2.5：跨屏时保持
    /// 逻辑尺寸不变）。主屏 scale = 1.0，逐字段原样返回，视觉零变化。
    pub fn scaled(&self, s: f32) -> LiquidParams {
        if (s - 1.0).abs() < 1e-4 {
            return *self;
        }
        let mut p = *self;
        p.corner_radius *= s;
        p.bevel_width *= s;
        p.thickness *= s;
        p.blur_max *= s;
        p.blur_min *= s;
        p.mouse_radius *= s;
        p.mouse_spread *= s;
        p.rim_width *= s;
        p.shadow_margin *= s;
        p.sanitize();
        p
    }

    /// 供控制面板读取 / 写入时的夹紧，防止非法值进入 GPU。
    pub fn sanitize(&mut self) {
        self.corner_radius = self.corner_radius.clamp(0.0, 400.0);
        self.lens_coverage = self.lens_coverage.clamp(0.0, 1.0);
        self.bevel_width = self.bevel_width.clamp(1.0, 400.0);
        self.super_n = self.super_n.clamp(2.0, 10.0);

        self.ior = self.ior.clamp(1.0, 2.0);
        self.thickness = self.thickness.clamp(0.0, 600.0);
        self.refr_strength = self.refr_strength.clamp(0.0, 2.0);
        self.dispersion = self.dispersion.clamp(0.0, 1.0);
        self.absorption = self.absorption.clamp(0.0, 2.0);

        self.clarity = self.clarity.clamp(0.0, 1.0);
        self.blur_max = self.blur_max.clamp(0.0, 96.0);
        self.blur_min = self.blur_min.clamp(0.0, 48.0).min(self.blur_max);
        self.progressive_curve = self.progressive_curve.clamp(0.0, 8.0);
        self.edge_always = self.edge_always.clamp(0.0, 1.0);

        self.tint_opacity = self.tint_opacity.clamp(0.0, 1.0);
        self.adaptive_tint = self.adaptive_tint.clamp(0.0, 1.0);
        self.base_alpha = self.base_alpha.clamp(0.0, 1.0);
        self.env_tint_mix = self.env_tint_mix.clamp(0.0, 1.0);

        self.light_intensity = self.light_intensity.clamp(0.0, 4.0);
        self.spec_intensity = self.spec_intensity.clamp(0.0, 4.0);
        self.spec_power = self.spec_power.clamp(1.0, 512.0);
        self.spec_spread = self.spec_spread.clamp(0.0, 1.0);
        self.fresnel_f0 = self.fresnel_f0.clamp(0.0, 1.0);
        self.rim_width = self.rim_width.clamp(0.0, 16.0);
        self.caustic_gain = self.caustic_gain.clamp(0.0, 4.0);

        self.mouse_radius = self.mouse_radius.clamp(20.0, 1200.0);
        self.mouse_intensity = self.mouse_intensity.clamp(0.0, 4.0);
        self.mouse_spread = self.mouse_spread.clamp(4.0, 400.0);
        self.proximity_glow = self.proximity_glow.clamp(0.0, 2.0);

        self.shadow_opacity = self.shadow_opacity.clamp(0.0, 1.0);
        self.shadow_margin = self.shadow_margin.clamp(0.0, 96.0);
        self.lag_seconds = self.lag_seconds.clamp(0.0, 0.1);

        for c in self.tint_color.iter_mut() {
            *c = c.clamp(0.0, 1.0);
        }
        for c in self.mouse_color.iter_mut() {
            *c = c.clamp(0.0, 1.0);
        }
        let l = self.light_dir;
        let len = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt().max(1e-4);
        self.light_dir = [l[0] / len, l[1] / len, l[2] / len];
    }
}

impl LiquidParams {
    /// 一组**惰性**参数：折射 / 模糊 / 高光 / 焦散 / 鼠标光 / 阴影全部关闭。
    ///
    /// 透明玻璃模式用它做底座（再覆盖 `corner_radius` / `base_alpha` / `tint_color`），
    /// 这样渲染层只需要一份参数结构，脏区推导、DPI 换算、外扩边距逻辑完全复用。
    /// 着色器另外按 `mode` 走早退分支，这些字段根本不会进入计算。
    #[must_use]
    pub fn inert() -> Self {
        Self {
            corner_radius: 0.0,
            lens_coverage: 0.0,
            bevel_width: 1.0,
            shape: Shape::RoundedRect,
            super_n: 4.0,
            profile: Profile::Circular,

            ior: 1.0,
            thickness: 0.0,
            refr_strength: 0.0,
            dispersion: 0.0,
            absorption: 0.0,

            clarity: 1.0,
            blur_max: 0.0,
            blur_min: 0.0,
            progressive_curve: 1.0,
            edge_always: 0.0,

            tint_opacity: 1.0,
            adaptive_tint: 0.0,
            base_alpha: 1.0,
            tint_color: [1.0, 1.0, 1.0, 1.0],
            env_tint_mix: 0.0,

            light_dir: [0.0, 0.0, 1.0],
            light_intensity: 0.0,
            spec_intensity: 0.0,
            spec_power: 1.0,
            spec_spread: 0.0,
            fresnel_f0: 0.0,
            rim_width: 0.0,
            caustic_gain: 0.0,

            // mouse_radius 取 sanitize 下限 20：它只在 mouse_intensity>0 时才会
            // 进影响盒（见 collect_monitor_blocks），这里留下限值只是为了
            // 「上层把强度调回非 0」时有一个合理的默认半径。
            mouse_radius: 20.0,
            mouse_intensity: 0.0,
            mouse_spread: 4.0,
            mouse_color: [1.0, 1.0, 1.0],
            proximity_glow: 0.0,

            shadow_opacity: 0.0,
            shadow_margin: 0.0,
            lag_seconds: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LiquidParams, Profile};

    #[test]
    fn sanitize_clamps_every_family() {
        let mut p = LiquidParams {
            clarity: 7.0,
            ior: 9.0,
            mouse_radius: 1e6,
            blur_max: 1e4,
            spec_power: 0.0,
            lens_coverage: -3.0,
            shadow_margin: 1e5,
            ..Default::default()
        };
        p.sanitize();
        assert!((0.0..=1.0).contains(&p.clarity));
        assert!((1.0..=2.0).contains(&p.ior));
        assert!((20.0..=1200.0).contains(&p.mouse_radius));
        assert!((0.0..=96.0).contains(&p.blur_max));
        assert!((1.0..=512.0).contains(&p.spec_power));
        assert!((0.0..=1.0).contains(&p.lens_coverage));
        assert!((0.0..=96.0).contains(&p.shadow_margin));
    }
    #[test]
    fn sanitize_keeps_blur_min_le_max() {
        let mut p = LiquidParams {
            blur_min: 900.0,
            blur_max: 3.0,
            ..Default::default()
        };
        p.sanitize();
        assert!(p.blur_min <= p.blur_max);
    }
    #[test]
    fn scaled_is_identity_at_unit_scale() {
        let p = LiquidParams::default();
        let q = p.scaled(1.0);
        assert!((q.mouse_radius - p.mouse_radius).abs() < 1e-6);
        assert!((q.corner_radius - p.corner_radius).abs() < 1e-6);
    }
    #[test]
    fn scaled_scales_pixel_dims() {
        let p = LiquidParams::default();
        let q = p.scaled(2.0);
        assert!((q.mouse_radius - p.mouse_radius * 2.0).abs() < 1e-3);
        assert!((q.shadow_margin - p.shadow_margin * 2.0).abs() < 1e-3);
        // 无量纲不变
        assert!((q.clarity - p.clarity).abs() < 1e-6);
    }
    #[test]
    fn inert_params_disable_every_effect() {
        let p = LiquidParams::inert();
        assert_eq!(p.refr_strength, 0.0);
        assert_eq!(p.dispersion, 0.0);
        assert_eq!(p.absorption, 0.0);
        assert_eq!(p.caustic_gain, 0.0);
        assert_eq!(p.spec_intensity, 0.0);
        assert_eq!(p.mouse_intensity, 0.0);
        assert_eq!(p.shadow_opacity, 0.0);
        assert_eq!(p.shadow_margin, 0.0);
        assert_eq!(p.clarity, 1.0, "透明模式不采样模糊层，clarity 必须是全清");
        // inert 必须本来就是 sanitize 的不动点
        let mut q = p;
        q.sanitize();
        assert_eq!(p, q);
    }

    #[test]
    fn params_is_copy_and_sanitize_is_idempotent() {
        let p = LiquidParams::default();
        let q = p; // Copy，不是 move（热路径逐块投影靠这个避免堆分配）
        assert_eq!(p, q);
        // sanitize 会把 light_dir 归一化成单位向量，所以 Default **不是**它的不动点。
        // 真正的不变量是幂等：连做两次结果必须完全一致。否则每帧重算参数会持续漂移，
        // 表现出来就是玻璃亮度缓慢变化（且极难归因）。
        let mut once = p;
        once.sanitize();
        let mut twice = once;
        twice.sanitize();
        assert_eq!(once, twice, "sanitize 必须幂等");
        let n = (once.light_dir[0] * once.light_dir[0]
            + once.light_dir[1] * once.light_dir[1]
            + once.light_dir[2] * once.light_dir[2])
            .sqrt();
        assert!((n - 1.0).abs() < 1e-5, "光照方向必须归一化，实际 |v|={n}");
    }

    #[test]
    fn profile_roundtrips_and_default_is_circular() {
        assert_eq!(LiquidParams::default().profile, Profile::Circular);
        let p = LiquidParams {
            profile: Profile::Parabolic,
            ..Default::default()
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: LiquidParams = serde_json::from_str(&json).unwrap();
        assert_eq!(back.profile, Profile::Parabolic);
    }
}
