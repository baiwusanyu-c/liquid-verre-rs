# liquid-glass —— Windows 桌面液态玻璃

严格按《折射渲染方案》v1.1 实现的 Windows 桌面「液态玻璃块」叠加层。
原生 **WGC + D3D11 + DirectComposition** 管线，不依赖任何 WebView / 前端运行时。

## 架构

```
WGC(Graphics Capture API) 采集主显示器
    -> 1 份共享 D3D11 纹理（CreateFreeThreaded，零拷贝包装）
    -> 全屏共享模糊金字塔（dual-filter 4 级）+ 块分辨率模糊层 + 1/16 环境色探针
    -> 单全屏画布（DirectComposition）+ 每个玻璃块一次 scissor 绘制
    -> 一次 Present + 一次 Commit
```

### 为什么 20+ 块不卡顿

| 关键点 | 实现 |
|---|---|
| 整屏只采集一次 | `src/capture/wgc.rs`，采集成本 O(屏幕)，与块数无关 |
| 模糊只算一次 | `src/gfx/pyramid.rs`，全屏共享，与块数无关 |
| 每块只画自己的矩形 | `src/gfx/glass_renderer.rs`，per-block viewport + scissor |
| 位移交给 DWM | 拖动只改窗口/块坐标，不重建位图 |
| 单设备单交换链 | 一次 Present / 一次 DComp Commit，与块数无关 |
| 自反馈根治 | `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` |

### 渲染管线（逐帧）

- **Pass A**（全屏一次）
  - 下采样：采集纹理 → 1/2 → 1/4 → 1/8 → 1/16（4-tap 角点，`shaders/blur_down.hlsl`）
  - 上采样：1/16 → 1/8 → 1/4 → 1/2（8-tap tent，`shaders/blur_up.hlsl`）
  - 块分辨率模糊层：整屏 → 1/2 → 1/2 → 整屏（强模糊 `frost`），另存轻模糊 `soft`
    > 这一步是相对方案的补强：1/16 层被放大到块尺寸会出现方块颗粒，
    > 因此在块像素分辨率上再生成一对模糊层，由 `radiusPx` 做二维插值。
  - 环境色探针：1/16 副本，供自适应染色（GPU 侧，不回读）
- **Pass B**（每块一次，`shaders/glass_ps.hlsl`）
  - SDF：圆角矩形（精确欧氏）或 squircle（超椭圆）
  - 斜面 LUT：`[slope, tanTheta2]` 256 项 `StructuredBuffer`，CPU 侧由 `src/glass/sdf.rs` 生成
  - Snell 折射：`offset_px = -grad * tanTheta2 * thickness * refr_strength`
  - RGB 色散：`tan2 * (1 ∓ δ)` 三次采样
  - 毛玻璃 ↔ 全透：`clarity` 控制模糊量与清晰混合，边缘始终保留折射（`edgeAlways`）
  - Beer-Lambert 吸收、Fresnel-Schlick、焦散亮带、鼠标点光、自适应染色
  - LOD-3：<32×32 的块关闭折射/焦散/鼠标光，只保留模糊 + 描边 + 高光

## 中文控制面板

启动后自动出现「液态玻璃 · 控制面板」（Win32 原生控件）。

- **玻璃块**：宽度 / 高度输入框；`＋ 添加一块`、`＋ 添加 20 块`、`删除最后一块`、`清空全部`
- **折射与光学参数**：全部通过 **slider（Trackbar）** 实时调节
  - 圆角半径、透镜覆盖、折射率 IOR、玻璃厚度、折射强度、色散
  - 清晰度、模糊半径
  - 高光强度、高光锐度、Fresnel、焦散增益
  - 染色浓度、自适应染色
  - 鼠标高光、鼠标半径
  - 阴影强度

交互：

- 拖动玻璃块任意位置移动；右下角拖拽缩放
- 点击某块即选中，面板的宽度/高度/圆角作用于该块
- 快捷键：`+` 增一块、`-` 删一块、`Esc` 退出

## 运行

前置：Rust stable（1.77+）、Windows 10 2004+ / Windows 11。

```powershell
cargo run --release
```

### 环境变量

| 变量 | 作用 |
|---|---|
| `GLASS_VISIBLE=1` | 关闭抓屏排除，便于截图 / 录屏验证视觉 |
| `GLASS_SHOT=<ms>` | 启动后指定毫秒抓一张整屏 BMP 到 `%TEMP%\liquid-glass-shot.bmp` |

## 项目结构

```
liquid-glass/
├─ Cargo.toml
├─ build.rs
├─ shaders/
│  ├─ glass_vs.hlsl         # 逐块矩形全屏三角形，SV_Position 为整屏像素坐标
│  ├─ glass_ps.hlsl         # SDF + Snell 折射 + 色散 + Fresnel + 焦散 + 鼠标光
│  ├─ blur_down.hlsl        # 4-tap 角点下采样
│  ├─ blur_up.hlsl          # 8-tap tent 上采样
│  └─ reduce_env.hlsl       # 环境色探针
└─ src/
   ├─ main.rs               # 窗口、消息循环、每帧调度
   ├─ capture/              # WGC 采集 + 最新帧槽位
   ├─ gfx/                  # 设备 / 着色器 / 金字塔 / 逐块绘制 / DComp / 交换链
   ├─ glass/                # 参数、SDF/LUT、块模型、管理器
   ├─ input/                # 光标轮询、拖拽状态机
   └─ ui/                   # 中文控制面板 + 开发期截图辅助
```

## 实测（本机 3120×2080，20 块）

| 指标 | 实测 |
|---|---|
| D3D11 设备 | 硬件（`warp=false`） |
| 渲染 / 采集帧 | 1200 / 568（12 秒内持续推进） |
| CPU | ≈0.4% 单核当量 |
| Release 进程 RSS | ≈67 MB |
| Release 二进制 | 0.38 MB |
