//! 玻璃块数据模型（方案 3.5 glass/block.rs）。

#[derive(Debug, Clone)]
pub struct Block {
    pub id: u32,
    pub label: String,
    /// 块左上角（逻辑 px，相对显示器工作区）
    pub x: f32,
    pub y: f32,
    /// 块尺寸（逻辑 px，不含阴影边距）
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    pub z_order: u32,
    /// 每个块独立的参数覆盖（None = 用全局）
    pub overrides: Option<crate::glass::params::Params>,
}

impl Block {
    pub fn new(id: u32, x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            id,
            label: format!("GLASS {id:02}"),
            x,
            y,
            width: width.max(MIN_SIZE),
            height: height.max(MIN_SIZE),
            radius: 28.0,
            z_order: id,
            overrides: None,
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
