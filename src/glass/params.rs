//! 全部可调参数（方案 5.5 / 5.6 / 7.x）。热更新只改常量缓冲区，不重建纹理。

use serde::{Deserialize, Serialize};

/// 斜面剖面类型（方案 5.4 的 1D LUT 剖面表）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    /// 圆形：sqrt(1-(1-u)^2)，边缘圆滑、放大感强（默认）
    Circular,
    /// 抛物线：u^2，位移线性，性能最好
    Parabolic,
    /// 正弦：sin(u*pi/2)，过渡最柔和
    Sinusoidal,
}

impl Default for Profile {
    fn default() -> Self {
        Profile::Circular
    }
}

impl Profile {
    /// H(u)，u: 0 边缘 → 1 平面
    pub fn height(self, u: f32) -> f32 {
        let u = u.clamp(0.0, 1.0);
        match self {
            Profile::Circular => {
                let x = 1.0 - u;
                (1.0 - x * x).max(0.0).sqrt()
            }
            Profile::Parabolic => u * u,
            Profile::Sinusoidal => (u * std::f32::consts::FRAC_PI_2).sin(),
        }
    }

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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    /// 圆角矩形（精确欧氏 SDF）
    RoundedRect,
    /// 超椭圆 squircle（曲率连续）
    Squircle,
}

impl Default for Shape {
    fn default() -> Self {
        Shape::RoundedRect
    }
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

/// 所有外观 / 交互参数。控制面板通过 IPC 直接改这些值。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Params {
    // ---- 周期 / 几何 ----
    /// 圆角半径（逻辑 px）
    pub corner_radius: f32,
    /// 斜面覆盖：0 = 仅边缘细带，1 = 覆盖整块（lens_coverage）
    pub lens_coverage: f32,
    /// 边缘斜面带宽（逻辑 px）
    pub bevel_width: f32,
    /// 形状模式
    pub shape: Shape,
    /// 面板超椭圆指数
    pub super_n: f32,

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

impl Default for Params {
    fn default() -> Self {
        Self {
            // 方案 v1.1 推荐默认值：Apple 风格液态玻璃
            // 对齐 liquid-glass-rs FrostedLiquid 预设，同时保留附件中的物理折射参数。
            corner_radius: 32.0,
            lens_coverage: 0.15,
            bevel_width: 26.0,
            shape: Shape::RoundedRect,
            super_n: 4.0,

            ior: 1.50,
            thickness: 26.0,
            refr_strength: 1.0,
            dispersion: 0.25,
            absorption: 0.18,

            clarity: 0.10,
            blur_max: 20.0,
            blur_min: 3.0,
            progressive_curve: 1.6,
            edge_always: 0.55,

            // Web 参考实现没有黑色面板底色；仅保留极轻的乳白 tint。
            tint_opacity: 0.018,
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

impl Params {
    /// 按显示器 DPI 比例缩放所有「像素量纲」的参数（方案 12.2.5：跨屏时保持
    /// 逻辑尺寸不变）。主屏 scale = 1.0，逐字段原样返回，视觉零变化。
    pub fn scaled(&self, s: f32) -> Params {
        if (s - 1.0).abs() < 1e-4 {
            return self.clone();
        }
        let mut p = self.clone();
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
