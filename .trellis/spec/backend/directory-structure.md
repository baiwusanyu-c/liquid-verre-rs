# 目录结构与职责边界

```
src/
  main.rs          # 主循环（消息泵/采集调度/脏区/质量控制器 QualityCtrl/面板同步）；不含像素逻辑
  capture/
    mod.rs         # Capture 统一封装 + 后端选择（HDR 规则见 rendering-pipeline §4）+ SYNTH 覆盖
    dda.rs         # DXGI Desktop Duplication：脏矩形、局部拷贝、sanitize_rect、verify 回读
    wgc.rs         # Windows.Graphics.Capture：帧池、B8G8R8A8 请求（HDR 由 DWM 转 SDR）
  gfx/
    compositor.rs  # 每屏画布：脏区书本账、金字塔有效性账本、清屏/Pass A 调度/Present/Commit、取证落盘
    pyramid.rs     # 共享模糊金字塔：下采样×4、frost ping-pong、env EMA、scissor 幂等外扩常量
    glass_renderer.rs # Pass B：per-block CB 打包（c0..c17 语义见 rendering-pipeline §3）、z 序绘制
    swapchain.rs   # CreateSwapChainForComposition（premultiplied alpha）
    dcomp.rs       # DComp visual/commit
    device.rs      # 设备/交换链延迟（GLASS_FRAME_LATENCY）/通用状态
    shaders.rs     # HLSL include_str! + 编译 + RT/SRV/cbuffer/sampler 工厂
    gpu_timer.rs   # 离线归因用 timestamp（禁止进闭环）
  glass/
    params.rs      # Params（全部光学/鼠标参数）+ sanitize + scaled(DPI) + cursor_influence_px
    block.rs       # Block 几何 + clamp（MIN/MAX_SIZE、半径、半出屏）
    manager.rs     # 块集合/z-order/命中测试/跨屏归属
    sdf.rs         # 折射 LUT 生成 + clamp_for_block（方案 12.9 上限）+ is_lod3
  input/
    cursor.rs      # GetCursorPos 轮询（零钩子，方案 6.3）
    drag.rs        # DragState：速度 EMA(0.75/0.25)、Move/ResizeSE、scale 换算
  ui/panel.rs      # 中文控制面板（原子量共享参数 → 主线程 apply）
shaders/           # HLSL：glass_vs/ps、blur_vs/down/up、env_ema（reduce_env.hlsl 为死文件，待删）
tools/             # 质量阀门与取证脚本（见 measurement-and-gates.md）；不入库产物已 gitignore
```

边界规则：像素/几何数学只允许在 `gfx/`、`glass/`、`shaders/`；`main.rs` 只做调度与书本账；
采集细节不得泄漏到 `gfx/`（经 `Capture` 封装）；面板不得直接写 GPU 资源（经原子量 + apply）。
