//! 纯逻辑领域模型：**不依赖 Win32 / GPU**，因此全部可单测。
//!
//! - [`params`]：液态玻璃全参数表
//! - [`block`] / [`manager`]：块记录与集合（含 z 序、命中测试）
//! - [`handle`]：8 方向缩放把手的几何判定
//! - [`draw`]：按 DPI 展开后的渲染投影块
//! - [`style`]：样式缓存（避免每帧重算 `sanitize`）
//! - [`geometry`]：矩形 / 影响盒运算的单一来源
//! - [`quality`]：自适应质量控制器
//! - [`sdf`]：折射 LUT 与逐块参数夹紧

pub(crate) mod block;
pub(crate) mod draw;
pub(crate) mod geometry;
pub(crate) mod handle;
pub(crate) mod manager;
pub(crate) mod params;
pub(crate) mod quality;
pub(crate) mod sdf;
pub(crate) mod style;
