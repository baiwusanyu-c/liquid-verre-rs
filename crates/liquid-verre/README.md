# liquid-verre-rs（crate 文档）

Windows 桌面**玻璃窗口组件库**：液态玻璃（WGC/DDA 采集 + D3D11 模糊金字塔 +
DirectComposition 合成 + Snell 折射）与透明玻璃两种模式，供上层应用做弹窗 / 悬浮面板底板。

完整文档（API 速查、层级语义、线程模型、环境变量表、构建与发布）在仓库根目录的
[README.md](../../README.md)；变更历史见 [CHANGELOG.md](../../CHANGELOG.md)。

## 快速开始

```rust,no_run
use liquid_verre::{Engine, EngineConfig, GlassStyle, WindowConfig};

let mut engine = Engine::spawn(EngineConfig::default())?;
let popup = engine.create_window(WindowConfig {
    rect: [320.0, 200.0, 560.0, 360.0],
    style: Some(GlassStyle::transparent(28.0, 0.40, [1.0, 1.0, 1.0, 1.0])),
    ..Default::default()
})?;
popup.set_corner_radius(40.0);
# Ok::<(), liquid_verre::Error>(())
```

## 平台

仅支持 Windows（D3D11 + DXGI Desktop Duplication / WGC + DirectComposition）。
跨平台项目请用 target 限定依赖：

```toml
[target.'cfg(windows)'.dependencies]
liquid-verre-rs = "0.4"
```
