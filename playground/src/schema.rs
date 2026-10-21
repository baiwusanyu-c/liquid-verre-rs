//! 面板控件的**单一来源**：字段名 / 中文名 / 取值范围 / 分组。
//!
//! 为什么要有这一层：需求 F1 要求「控制面板支持的所有调节全部参数化」。
//! 如果 HTML 里手写一份控件、Rust 里手写一份 `Params`，两边迟早漂移
//! （重构前的面板就只暴露了 40 个参数里的 17 个）。
//! 现在前端完全按这份 schema 动态生成控件，**加一个参数只需要在这里加一行**。

use serde::Serialize;

/// 控件类型。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum FieldKind {
    /// 连续滑杆。
    Float { min: f32, max: f32, step: f32 },
    /// RGBA 颜色（4 个 0–1 通道）。
    Color4,
    /// RGB 颜色（3 个 0–1 通道）。
    Color3,
    /// 三维向量（例如光照方向）。
    Vec3,
    /// 枚举下拉。
    Enum { options: &'static [&'static str] },
    /// 布尔开关。
    Bool,
}

/// 一个可调字段。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    /// JSON 字段名（与 `LiquidParams` 的 serde 名一致，camelCase）。
    pub key: &'static str,
    /// 面板上显示的中文名。
    pub label: &'static str,
    /// 分组名。
    pub group: &'static str,
    /// 控件类型。
    #[serde(flatten)]
    pub kind: FieldKind,
}

/// 滑杆字段的简写构造器。
///
/// 用 `fn` 而不是 `const` 闭包：`const` 项必须显式标注类型，
/// 而闭包类型不可写，编译器会直接报 `missing type for const item`。
fn f(
    key: &'static str,
    label: &'static str,
    group: &'static str,
    min: f32,
    max: f32,
    step: f32,
) -> Field {
    Field {
        key,
        label,
        group,
        kind: FieldKind::Float { min, max, step },
    }
}

/// 液态玻璃模式的全部可调参数（对应 `LiquidParams` 的每个字段）。
pub fn liquid_fields() -> Vec<Field> {
    vec![
        // ---- 几何 / 形状 ----
        f("cornerRadius", "圆角半径", "几何", 0.0, 400.0, 1.0),
        f("lensCoverage", "斜面覆盖", "几何", 0.0, 1.0, 0.01),
        f("bevelWidth", "斜面带宽", "几何", 1.0, 400.0, 1.0),
        Field {
            key: "shape",
            label: "形状",
            group: "几何",
            kind: FieldKind::Enum {
                options: &["RoundedRect", "Squircle"],
            },
        },
        f("superN", "超椭圆指数", "几何", 2.0, 10.0, 0.1),
        Field {
            key: "profile",
            label: "斜面剖面",
            group: "几何",
            kind: FieldKind::Enum {
                options: &["Circular", "Parabolic", "Sinusoidal"],
            },
        },
        // ---- 折射 ----
        f("ior", "折射率 IOR", "折射", 1.0, 2.0, 0.01),
        f("thickness", "等效厚度", "折射", 0.0, 600.0, 1.0),
        f("refrStrength", "折射强度", "折射", 0.0, 2.0, 0.01),
        f("dispersion", "RGB 色散", "折射", 0.0, 1.0, 0.01),
        f("absorption", "吸收(Beer)", "折射", 0.0, 2.0, 0.01),
        // ---- 毛玻璃 ----
        f("clarity", "清晰度", "毛玻璃", 0.0, 1.0, 0.01),
        f("blurMax", "最大模糊半径", "毛玻璃", 0.0, 96.0, 0.5),
        f("blurMin", "最小模糊半径", "毛玻璃", 0.0, 48.0, 0.5),
        f("progressiveCurve", "渐进曲线", "毛玻璃", 0.0, 8.0, 0.05),
        f("edgeAlways", "边缘保持清晰", "毛玻璃", 0.0, 1.0, 0.01),
        // ---- 染色 ----
        f("tintOpacity", "染色不透明度", "染色", 0.0, 1.0, 0.005),
        f("adaptiveTint", "自适应染色", "染色", 0.0, 1.0, 0.01),
        f("baseAlpha", "基础 alpha", "染色", 0.0, 1.0, 0.01),
        Field {
            key: "tintColor",
            label: "染色颜色",
            group: "染色",
            kind: FieldKind::Color4,
        },
        f("envTintMix", "环境色混入", "染色", 0.0, 1.0, 0.01),
        // ---- 光照 ----
        Field {
            key: "lightDir",
            label: "主光方向",
            group: "光照",
            kind: FieldKind::Vec3,
        },
        f("lightIntensity", "主光强度", "光照", 0.0, 4.0, 0.01),
        f("specIntensity", "高光强度", "光照", 0.0, 4.0, 0.01),
        f("specPower", "高光锐度", "光照", 1.0, 512.0, 1.0),
        f("specSpread", "高光扩散", "光照", 0.0, 1.0, 0.01),
        f("fresnelF0", "Fresnel F0", "光照", 0.0, 1.0, 0.005),
        f("rimWidth", "描边宽度", "光照", 0.0, 16.0, 0.1),
        f("causticGain", "焦散增益", "光照", 0.0, 4.0, 0.01),
        // ---- 鼠标点光 ----
        f("mouseRadius", "鼠标半径", "鼠标", 20.0, 1200.0, 1.0),
        f("mouseIntensity", "鼠标强度", "鼠标", 0.0, 4.0, 0.01),
        f("mouseSpread", "鼠标高光扩散", "鼠标", 4.0, 400.0, 1.0),
        Field {
            key: "mouseColor",
            label: "鼠标高光颜色",
            group: "鼠标",
            kind: FieldKind::Color3,
        },
        f("proximityGlow", "邻近余晖", "鼠标", 0.0, 2.0, 0.01),
        // ---- 阴影 / 交互 ----
        f("shadowOpacity", "阴影不透明度", "其它", 0.0, 1.0, 0.01),
        f("shadowMargin", "阴影边距", "其它", 0.0, 96.0, 1.0),
        f("lagSeconds", "拖拽滞后补偿(s)", "其它", 0.0, 0.1, 0.001),
    ]
}

/// 透明玻璃模式的全部可调参数（需求 F3.2：**只有这三项**）。
pub fn transparent_fields() -> Vec<Field> {
    vec![
        f("cornerRadius", "圆角半径", "透明玻璃", 0.0, 400.0, 1.0),
        f("opacity", "玻璃透明度", "透明玻璃", 0.0, 1.0, 0.01),
        Field {
            key: "color",
            label: "玻璃颜色",
            group: "透明玻璃",
            kind: FieldKind::Color4,
        },
    ]
}

/// 缩放交互（需求 F2 / F2.3）。
pub fn resize_fields() -> Vec<Field> {
    vec![
        Field {
            key: "enabled",
            label: "允许缩放",
            group: "缩放",
            kind: FieldKind::Bool,
        },
        Field {
            key: "edges",
            label: "边缘拖拽",
            group: "缩放",
            kind: FieldKind::Bool,
        },
        Field {
            key: "corners",
            label: "对角拖拽",
            group: "缩放",
            kind: FieldKind::Bool,
        },
        f("gripPx", "命中带宽", "缩放", 2.0, 128.0, 1.0),
        f("minWidth", "最小宽度", "缩放", 8.0, 2000.0, 1.0),
        f("minHeight", "最小高度", "缩放", 8.0, 2000.0, 1.0),
        Field {
            key: "feedback.enabled",
            label: "border 视觉反馈",
            group: "缩放",
            kind: FieldKind::Bool,
        },
        f(
            "feedback.borderWidth",
            "border 粗细",
            "缩放",
            0.5,
            32.0,
            0.5,
        ),
        f("feedback.alpha", "border 不透明度", "缩放", 0.0, 1.0, 0.01),
        Field {
            key: "feedback.color",
            label: "border 颜色",
            group: "缩放",
            kind: FieldKind::Color3,
        },
    ]
}

/// 校验：schema 里的每个 key 都必须真的存在于 `LiquidParams` 的 JSON 里，
/// 否则面板上会出现「拖了没反应」的死控件。
#[cfg(test)]
mod tests {
    use super::*;
    use liquid_verre::{LiquidParams, ResizeOptions, TransparentParams};

    fn keys_of<T: serde::Serialize>(v: &T) -> Vec<String> {
        let json = serde_json::to_value(v).unwrap();
        json.as_object().unwrap().keys().cloned().collect()
    }

    #[test]
    fn every_liquid_field_exists_in_params() {
        let have = keys_of(&LiquidParams::default());
        for f in liquid_fields() {
            assert!(
                have.contains(&f.key.to_string()),
                "面板字段 {:?} 在 LiquidParams 里不存在（现有: {have:?}）",
                f.key
            );
        }
    }

    #[test]
    fn every_param_is_exposed_in_the_panel() {
        // 需求 F1 的反向保证：不能出现「库支持但面板调不到」的参数
        let have = keys_of(&LiquidParams::default());
        let shown: Vec<String> = liquid_fields().iter().map(|f| f.key.to_string()).collect();
        for k in have {
            assert!(
                shown.contains(&k),
                "LiquidParams::{k} 没有出现在面板 schema 里"
            );
        }
    }

    #[test]
    fn transparent_exposes_exactly_three_fields() {
        let have = keys_of(&TransparentParams::default());
        let shown: Vec<String> = transparent_fields()
            .iter()
            .map(|f| f.key.to_string())
            .collect();
        assert_eq!(shown.len(), 3, "需求 F3.2：透明模式只有三个参数");
        for k in have {
            assert!(shown.contains(&k), "TransparentParams::{k} 未暴露");
        }
    }

    #[test]
    fn resize_fields_cover_every_option() {
        let opts = ResizeOptions::default();
        let json = serde_json::to_value(opts).unwrap();
        let shown: Vec<String> = resize_fields().iter().map(|f| f.key.to_string()).collect();
        for k in keys_of(&opts) {
            // `feedback` 是嵌套对象，由下面的 feedback.* 检查覆盖，这里跳过容器本身
            if k == "feedback" {
                continue;
            }
            assert!(shown.contains(&k), "ResizeOptions::{k} 未暴露");
        }
        let fb = json.get("feedback").unwrap().as_object().unwrap();
        for k in fb.keys() {
            assert!(
                shown.contains(&format!("feedback.{k}")),
                "feedback.{k} 未暴露"
            );
        }
    }

    #[test]
    fn ranges_are_sane_and_ordered() {
        for f in liquid_fields()
            .iter()
            .chain(transparent_fields().iter())
            .chain(resize_fields().iter())
        {
            if let FieldKind::Float { min, max, step } = f.kind {
                assert!(min < max, "{}: min>=max", f.key);
                assert!(step > 0.0, "{}: step<=0", f.key);
                assert!(step <= (max - min), "{}: step 大于值域", f.key);
            }
        }
    }
}
