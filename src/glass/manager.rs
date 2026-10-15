//! 块的增删改与布局（方案 3.5 / 6.x）。

use crate::glass::block::Block;
use crate::glass::params::Params;

pub struct Manager {
    blocks: Vec<Block>,
    next_id: u32,
}

impl Default for Manager {
    fn default() -> Self {
        Self {
            blocks: Vec::new(),
            next_id: 1,
        }
    }
}

impl Manager {
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }
    pub fn blocks_mut(&mut self) -> &mut Vec<Block> {
        &mut self.blocks
    }
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn add(&mut self, x: f32, y: f32, width: f32, height: f32) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        let mut b = Block::new(id, x, y, width, height);
        b.clamp();
        self.blocks.push(b);
        id
    }

    /// 级联自动布局：按屏幕尺寸错位排布，避免重叠。
    pub fn add_auto(
        &mut self,
        index: usize,
        width: f32,
        height: f32,
        screen_w: f32,
        screen_h: f32,
    ) -> u32 {
        let gap = 22.0;
        let step_x = (width * 0.62 + gap).max(96.0);
        let step_y = (height * 0.60 + gap).max(84.0);
        let cols = (((screen_w - 40.0) / step_x).floor() as usize).clamp(1, 8);
        let col = index % cols;
        let row = (index / cols) % 12;
        let x = (32.0 + col as f32 * step_x).min((screen_w - width - 24.0).max(8.0));
        let y = (64.0 + row as f32 * step_y).min((screen_h - height - 32.0).max(8.0));
        self.add(x, y, width, height)
    }

    pub fn remove(&mut self, id: u32) -> bool {
        let before = self.blocks.len();
        self.blocks.retain(|b| b.id != id);
        self.blocks.len() != before
    }

    pub fn clear(&mut self) {
        self.blocks.clear();
    }

    pub fn get(&self, id: u32) -> Option<&Block> {
        self.blocks.iter().find(|b| b.id == id)
    }

    pub fn get_mut(&mut self, id: u32) -> Option<&mut Block> {
        self.blocks.iter_mut().find(|b| b.id == id)
    }

    /// 命中测试：返回最上层（z 最大）被命中的块
    pub fn hit_test(&self, x: f32, y: f32) -> Option<u32> {
        self.blocks
            .iter()
            .filter(|b| x >= b.x && x <= b.x + b.width && y >= b.y && y <= b.y + b.height)
            .max_by_key(|b| b.z_order)
            .map(|b| b.id)
    }

    pub fn bring_to_front(&mut self, id: u32, params: &Params) {
        let _ = params;
        let max_z = self.blocks.iter().map(|b| b.z_order).max().unwrap_or(0);
        if let Some(b) = self.get_mut(id) {
            if b.z_order < max_z {
                b.z_order = max_z + 1;
            }
        }
    }
}
