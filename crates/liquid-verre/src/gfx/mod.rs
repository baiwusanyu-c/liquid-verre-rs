//! D3D11 / DXGI / DirectComposition 渲染层。
//!
//! 边界：采集细节不得泄漏到本层（一律经 `crate::capture::Capture`）；
//! 本层**原则上**不读环境变量；已登记的例外全在构造期一次性读取
//! （`gfx/compositor.rs` 的 `GLASS_VERIFY_VALIDITY` / `GLASS_FORCE_FULL` / `GLASS_SKIP_*` /
//! `GLASS_SHOT_EVERY_MS` / `GLASS_INTERNAL_SHOT_AT_MS`，`gfx/pyramid.rs` 的
//! `GLASS_ENV_EMA` / `GLASS_FROST_HALF` / `GLASS_PASS_A_HZ`，`gfx/device.rs` 的
//! `GLASS_FRAME_LATENCY`），清单见 `diag/env.rs` 模块文档；它们都不进热路径。

pub(crate) mod compositor;
pub(crate) mod dcomp;
pub(crate) mod device;
pub(crate) mod glass_renderer;
pub(crate) mod gpu_timer;
pub(crate) mod pyramid;
pub(crate) mod shaders;
pub(crate) mod swapchain;

pub(crate) use compositor::Compositor;
pub(crate) use dcomp::DComp;
pub(crate) use device::Device;
pub(crate) use glass_renderer::GlassRenderer;
