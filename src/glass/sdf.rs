//! CPU 侧 SDF / Snell LUT 与参数夹紧（方案 5.3 / 5.4 / 12.9）。
//! LUT 只在参数变化时重建，运行期每像素仅 1 次查表 + 1 次插值。

use crate::glass::params::{Params, Profile, MAX_SLOPE};

pub const LUT_SIZE: usize = 256;

/// 生成 [slope(u), tanTheta2(u)] 两条 1D LUT。
pub fn build_refraction_lut(params: &Params, profile: Profile) -> Vec<[f32; 2]> {
    let eta = 1.0 / params.ior.max(1.0); // 空气 -> 玻璃
    (0..LUT_SIZE)
        .map(|i| {
            let u = i as f32 / (LUT_SIZE - 1) as f32; // 0 = 边缘, 1 = 平面
            let slope = profile.slope(u);
            // Snell：sinθ1 = slope / sqrt(1+slope^2)，sinθ2 = η·sinθ1
            let s1 = slope / (1.0 + slope * slope).sqrt();
            let s2 = (eta * s1).clamp(-1.0, 1.0);
            let c2 = (1.0 - s2 * s2).max(0.0).sqrt();
            let tan2 = if c2 > 1e-4 { s2 / c2 } else { MAX_SLOPE };
            [slope, tan2]
        })
        .collect()
}

/// 按块尺寸夹紧参数（方案 12.9）：任意尺寸下不出现自交 / 越界。
pub fn clamp_for_block(params: &mut Params, width: f32, height: f32) {
    // 块逻辑尺寸（不含阴影边距）
    let bw = (width - params.shadow_margin * 2.0).max(1.0);
    let bh = (height - params.shadow_margin * 2.0).max(1.0);
    let min_hs = (bw.min(bh) * 0.5).max(0.5);

    params.corner_radius = params.corner_radius.min(min_hs);
    params.bevel_width = params.bevel_width.min(0.45 * min_hs).max(1.0);
    // 位移夹紧到 ±0.35 * blockSize
    let max_offset = 0.35 * bw.min(bh);
    let max_strength = (max_offset / (params.thickness.max(1.0) * MAX_SLOPE)).clamp(0.05, 2.0);
    params.refr_strength = params.refr_strength.min(max_strength);
}

/// 小于 32x32 的块走 LOD-3：只保留模糊 + 描边 + 高光。
pub fn is_lod3(width: f32, height: f32) -> bool {
    width < 32.0 || height < 32.0
}

#[cfg(test)]
mod tests {
    use super::{build_refraction_lut, clamp_for_block, is_lod3, Profile};
    use crate::glass::params::Params;

    #[test]
    fn lod3_boundary() {
        assert!(is_lod3(31.9, 500.0));
        assert!(is_lod3(500.0, 31.9));
        assert!(!is_lod3(32.0, 32.0));
    }
    #[test]
    fn clamp_for_block_respects_spec_limits() {
        // 方案 12.9：bevel ≤ 0.45*min(hs)、radius ≤ min(hs)、位移 ≤ ±0.35*blockSize
        let mut p = Params::default();
        p.corner_radius = 10_000.0;
        p.bevel_width = 10_000.0;
        p.refr_strength = 10_000.0;
        p.thickness = 40.0;
        clamp_for_block(&mut p, 200.0, 120.0);
        let bw = (200.0 - p.shadow_margin * 2.0).max(1.0);
        let bh = (120.0 - p.shadow_margin * 2.0).max(1.0);
        let min_hs = bw.min(bh) * 0.5;
        assert!(p.corner_radius <= min_hs + 1e-3);
        assert!(p.bevel_width <= 0.45 * min_hs + 1e-3);
        assert!(p.bevel_width >= 1.0);
        assert!(p.refr_strength <= 2.0);
    }
    #[test]
    fn lut_is_monotone_and_finite() {
        let p = Params::default();
        let lut = build_refraction_lut(&p, Profile::Circular);
        assert_eq!(lut.len(), 256);
        for w in lut.windows(2) {
            assert!(w[0][0] >= w[1][0] - 1e-4, "slope not monotone at {:?}", w);
            assert!(w[0][1].is_finite() && w[1][1].is_finite());
        }
    }
}
