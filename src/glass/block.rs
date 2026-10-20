//! 玻璃块数据模型（方案 3.5 glass/block.rs）。

/// 全部字段都是标量，`Copy` 语义；多屏投影每帧复制块时不会产生堆分配
/// （方案 7.3「热路径零堆分配」）。
#[derive(Debug, Clone, Copy)]
pub struct Block {
    pub id: u32,
    /// 块左上角（虚拟屏物理 px，多屏绝对坐标）
    pub x: f32,
    pub y: f32,
    /// 块尺寸（所在屏物理 px，不含阴影边距）
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    pub z_order: u32,
    /// 所在显示器下标（方案 12.10：默认钳制到单屏，不跨屏）
    pub monitor: usize,
    /// 该屏 DPI / 主屏 DPI。块跨屏时按它换算，保持物理观感一致。
    pub scale: f32,
}

impl Block {
    pub fn new(id: u32, x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            id,
            x,
            y,
            width: width.max(MIN_SIZE),
            height: height.max(MIN_SIZE),
            radius: 28.0,
            z_order: id,
            monitor: 0,
            scale: 1.0,
        }
    }

    pub fn clamp(&mut self) {
        self.width = self.width.clamp(MIN_SIZE, MAX_SIZE);
        self.height = self.height.clamp(MIN_SIZE, MAX_SIZE);
        self.radius = self.radius.clamp(0.0, 0.5 * self.width.min(self.height));
        self.x = self.x.clamp(-self.width * 0.5, 20000.0);
        self.y = self.y.clamp(-self.height * 0.5, 20000.0);
    }
}

pub const MIN_SIZE: f32 = 24.0;
pub const MAX_SIZE: f32 = 8000.0;

#[cfg(test)]
mod tests {
    use super::{Block, MAX_SIZE, MIN_SIZE};

    #[test]
    fn clamp_enforces_size_and_radius_bounds() {
        let mut b = Block::new(1, 0.0, 0.0, 1e9, 1e9);
        b.radius = 1e9;
        b.clamp();
        assert_eq!(b.width, MAX_SIZE);
        assert_eq!(b.height, MAX_SIZE);
        assert!(b.radius <= 0.5 * b.width.min(b.height));
        let mut c = Block::new(2, 0.0, 0.0, 1.0, 1.0);
        c.clamp();
        assert_eq!(c.width, MIN_SIZE);
        assert_eq!(c.height, MIN_SIZE);
    }
    #[test]
    fn clamp_allows_half_offscreen_but_bounds_extreme() {
        let mut b = Block::new(3, -1e9, -1e9, 400.0, 300.0);
        b.clamp();
        assert!(b.x >= -b.width * 0.5);
        assert!(b.y >= -b.height * 0.5);
    }
}
