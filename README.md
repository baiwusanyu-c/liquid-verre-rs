# liquid-glass —— Windows 桌面液态玻璃

严格按《折射渲染方案》v1.1 实现的 Windows 桌面「液态玻璃块」叠加层。
原生 **WGC/DDA + D3D11 + DirectComposition** 管线，不依赖任何 WebView / 前端运行时。
**多显示器**：每块屏独立采集、独立金字塔、独立画布，分辨率与 DPI 各不相同也各自正确。

## 架构

```
每块显示器（EnumDisplayMonitors + GetDpiForMonitor）
  ├─ 1 个采集会话，按方案 10.5 的 pick_backend() 选后端：
  │     DDA（优先，提供脏矩形、延迟更低）-> 失败回退 WGC（支持并发采集）
  │     └─ DDA: 脏矩形 -> 只 CopySubresourceRegion 变化的区域，不整屏搬 26 MB
  │     └─ 失效（锁屏/独占全屏/被占用）时 500 ms 退避重建，恢复后自动继续
  ├─ 1 份该屏分辨率的共享 D3D11 纹理
  ├─ 1 套该屏的模糊金字塔，只刷新「DDA 脏矩形 ∩ 块足迹 ⊕ 采样余量」
  └─ 1 个全屏画布窗口（WS_EX_NOREDIRECTIONBITMAP + WDA_EXCLUDEFROMCAPTURE）
        └─ 每块玻璃一次 scissor 绘制，只画与脏区相交的块
        └─ 一次 Present + 一次 DComp Commit
```

### 为什么 20+ 块不卡顿

| 关键点 | 实现 |
|---|---|
| 整屏只采集一次 | `src/capture/dda.rs`，成本 O(屏幕)，与块数无关 |
| **只搬变化的像素** | `GetFrameDirtyRects` / `GetFrameMoveRects` -> 逐矩形 `CopySubresourceRegion` |
| **模糊只算块看得见的地方** | `src/gfx/pyramid.rs`，Pass A 的 scissor = 桌面脏区 ∩ 块足迹 |
| **只重画真的变了的块** | `src/gfx/compositor.rs`，Pass B 脏区 = 块位移 ∪ 鼠标点光影响范围 ∪ 桌面脏区 |
| 每块只画自己的矩形 | `src/gfx/glass_renderer.rs`，per-block viewport + scissor |
| 位移交给 DWM | 拖动只改块矩形，不重建位图、不 `ResizeBuffers` |
| 单设备、每屏一次 Present | 一次 Present / 一次 DComp Commit，与块数无关 |
| **没有玻璃的屏整帧短路** | 不清屏、不 Pass B、不 Present、不 Commit |
| **事件驱动** | 无触发源就整帧跳过；静止桌面不 Present、不 Commit |
| 自反馈根治 | `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`（DDA/WGC 均已验收：不是黑块） |
| **HDR 桌面** | HDR/宽色域输出（色彩空间 ≠ SDR G22）下 DDA 复制面是 HDR 参考值（整张桌面过曝）；自动改走 **WGC**，由 DWM 完成 HDR→SDR（实测与 SDR 桌面统计一致）。SDR 输出仍 DDA 优先；`GLASS_BACKEND` 可强制 |
| 拖拽期零 `SetWindowRgn` | 按下时一次性扩到整窗，抬起后收紧 |
| 不抢焦点 | 画布带 `WS_EX_NOACTIVATE`（方案 需求 2） |
| 独占全屏自动让路 | 无边框 popup 铺满显示器时隐藏该屏画布，退出后恢复（方案 R-16） |
| 环境色不闪烁 | 1/16 层 GPU ping-pong EMA，τ=150 ms，零回读（方案 12.6 / R-20） |
| **块与块之间零串扰** | 鼠标驱动的 border 渐变乘 `fall × 有限支撑窗(3R)`，脏区「光标影响盒」半径与之严格对齐：**不再被重画的块，其鼠标项精确为 0**（`tools/verify_mouse_isolation.py` 逐位取证） |
| 撑不住时自动降档 | 自适应质量控制器（方案 12.4.3，**默认开启**）：持续丢 vblank 才逐级降，恢复后逐级升回；level 0 与未启用时**逐位一致** |

### 渲染管线（逐帧）

- **门控**：新桌面帧 / 参数变化 / 几何变化 / hover 变化 / 光标进入点光影响半径 /
  拖拽 / 速度未收敛 / 窗口消息 —— 全为空则整帧跳过。
- **Pass A**（每屏，仅在需要时）
  - 下采样：采集纹理 -> 1/2 -> 1/4 -> 1/8 -> 1/16（4-tap 角点，`shaders/blur_down.hlsl`）
  - 块分辨率模糊层：整屏 -> 1/2 -> 1/2 ×4 次 tent -> 整屏（强模糊 `frost`）
    > `soft` 与 `frost` 内容逐像素相同，共享同一份 RT/SRV。
    > 1/16 层直接兼作环境色探针，不再额外拷贝。
  - 全程 scissor 到「桌面脏区 ∩ 块足迹」，与整屏无关。
- **Pass B**（每块一次，`shaders/glass_ps.hlsl`）
  - SDF：圆角矩形（精确欧氏）或 squircle（超椭圆）
  - 斜面 LUT：`[slope, tanTheta2]` 256 项 `StructuredBuffer`，CPU 侧由 `src/glass/sdf.rs` 生成
  - Snell 折射、RGB 色散、Beer-Lambert 吸收、Fresnel-Schlick、焦散亮带、鼠标点光、自适应染色
  - 毛玻璃 ↔ 全透：`clarity` 连续控制，边缘始终保留折射（`edgeAlways`）
  - LOD-3：<32×32 的块关闭折射/焦散/鼠标光

### 多显示器与 DPI

- 块几何以**主屏物理像素**为逻辑单位；落到 scale = `dpi / 主屏 dpi` 的屏上按该比例换算，
  物理观感保持一致（方案 12.2.5）。主屏 scale 恒为 1.0，因此主屏视觉与单屏版本逐位一致。
- 默认**钳制在单屏内**（方案 12.10 策略 ①）：块中心越过边界即换屏并按比例换算尺寸，
  不做跨屏大玻璃，因此不存在两屏采样不连续的接缝问题。
- `WM_DISPLAYCHANGE` / `WM_DPICHANGED` -> 重建全部画布与采集，块自动重新归位。
- 采集失效（锁屏安全桌面 / 独占全屏 / 被其他录屏软件抢占）时按 500 ms 退避重建，
  恢复后自动继续，主循环有速率下限，任何异常下都不会空转。

## 中文控制面板

启动后自动开启一个本地 HTTP 服务并用系统浏览器打开「液态玻璃 · 控制面板」。

- **玻璃块**：宽度 / 高度（逻辑单位，跨屏自动换算）；`＋ 添加一块`、`＋ 添加 20 块`、`删除最后一块`、`清空全部`
- **折射与光学参数**：圆角半径、透镜覆盖、折射率 IOR、玻璃厚度、折射强度、色散、
  清晰度、模糊半径、高光强度/锐度、Fresnel、焦散增益、染色浓度、自适应染色、
  鼠标高光/半径、阴影强度

交互：

- 拖动玻璃块任意位置移动；右下角拖拽缩放；可直接拖到另一块显示器
- 点击某块即选中，面板的宽度/高度/圆角作用于该块
- 快捷键：`+` 增一块、`-` 删一块、`Esc` 退出

## 运行

前置：Rust stable（1.77+）、Windows 10 2004+ / Windows 11。

```powershell
cargo run --release
```

### 环境变量（全部只影响诊断/布局，不影响视觉）

| 变量 | 作用 |
|---|---|
| `GLASS_METRICS=1` | 每 5 秒墙钟窗口输出 CPU / 脏区 / Pass A / DDA / 帧率统计 |
| `GLASS_GPU_TIMINGS=1` | 额外打开 D3D11 timestamp 计时。**会打断驱动批处理、显著拖慢管线**，只在专门做 GPU 归因时开 |
| `GLASS_MANY=1` | 启动即 20 块自动布局 |
| `GLASS_SPREAD=1` | 每块显示器正中各放一块玻璃（多屏验收） |
| `GLASS_FAR_GRID=1` | 4 块互不重叠的分离布局（跨块串扰取证用） |
| `GLASS_CURSOR_AT=x,y` | 固定首帧光标位置，使「光标在 A 处」与「光标在 B 处」两张取证图可逐像素对比 |
| `GLASS_FRAME_LATENCY=<1..8>` | DXGI 渲染队列深度，默认 1（方案 7.4-7，降低光标到画面的滞后） |
| `GLASS_FORCE_FULL=1` | 关闭 Pass B 脏矩形，每帧整屏重画（A/B 对照） |
| `GLASS_INTERNAL_SHOT=<path>` | 输出内部取证图（合成结果 / overlay / capture / frost） |
| `GLASS_INTERNAL_SHOT_AT_MS=<ms>` | 延迟取证，配合下面两项才能测到脏矩形路径 |
| `GLASS_VERIFY_PASS_A=1` | 同进程再渲一帧「全量 Pass A + 整屏 Pass B」，输出 `.full.bmp` 对照图 |
| `GLASS_VERIFY_COPY=1` | 并行维护整屏拷贝的参考纹理，周期性逐像素比对局部拷贝结果 |
| `GLASS_EXTREME_SPEC=1` | 极端高光回归（`spec_intensity=4.0`、`spec_power=1.0`） |
| `GLASS_BACKEND=dda\|wgc` | 强制指定采集后端（默认 DDA 优先、失败回退 WGC） |
| `GLASS_WGC_BUFFERS=<2..8>` | WGC 帧池缓冲数（方案 12.7：>90 Hz 用 3） |
| `GLASS_MONITORS=<n>` | 只启用前 n 块显示器 |
| `GLASS_ENV_EMA=0` | 关闭环境色时域平滑，回到逐帧直采 1/16 层 |
| `GLASS_VERIFY_VALIDITY=1` | 漏更新自检：金字塔有效性虚报 / 该重画却漏掉的块，只打印计数不改渲染 |
| `GLASS_FORCE_HDR_CONV=1` | SDR 源也强制走 HDR->SDR 转换路径（数值校验用） |
| `GLASS_SIM_BUSY=<ms>` | 拖拽期每帧注入 N ms 忙等，确定性地驱动控制器降档/回升（验证用） |
| `GLASS_ADAPTIVE=0` | 关闭自适应质量控制器（**默认开启**）。level 0 = 与关闭时逐位一致，只有持续丢帧才降档 |
| `GLASS_FROST_HALF=1` | 手动强制 frost 保持 1/2 分辨率（自适应控制器的 level 1）。差异 ≤3/255，非逐位一致 |
| `GLASS_SYNTH_CAP=1` | 用确定性合成图案覆盖采集结果，使跨进程/跨时间的逐像素取证可复现 |
| `GLASS_SKIP_PASS_A/PASS_B/PRESENT=1` | 归因开关：分别跳过某一段 GPU 工作（画面会不正确，仅实验用） |

### 自检脚本

```powershell
python tools/verify_all.py           # 7 项逐像素取证，期望 ALL PASS（含跨版本对比）
python tools/verify_adaptive.py      # 自适应控制器行为：降档/回升/显存回收
python tools/attribute.py            # 拖拽帧时间归因：逐项关掉 Pass A / Pass B / Present
python tools/perf_probe.py --case all --seconds 12 --warmup 4 --out tools/final.json
python tools/verify_dirty.py         # 脏矩形渲染 vs 全量渲染，逐像素（含真实拖拽）
python tools/verify_crossscreen.py   # 跨屏拖拽 + DPI 换算验收
python tools/verify_mouse_isolation.py # 拖一块，其余块必须 0 像素变化（含反向对照）
python tools/measure_frost_half.py   # frost 半分辨率档的视觉代价 + 性能收益量化
python tools/ab_memory.py            # 常驻内存受控 A/B（轮转交错 + 稳态窗口中位数）
python tools/ab_influence.py         # 光标影响盒修复前/后两个二进制的同环境 A/B
python tools/verify_validity.py        # 漏更新自检（金字塔虚报 / 漏画计数必须为 0）
```

自适应质量档位（方案 12.4.3，按**实测收益**排序）：

| level | 动作 | 触发条件 |
|---|---|---|
| 0 | 全部效果开启，与未启用控制器时逐位一致 | 默认 |
| 1 | frost 保持 1/2 分辨率（+14% fps；单屏 −27.4 MB / 双屏 −35.5 MB，受控 A/B 实测；模糊层差异 ≤3/255） | 见下 |
| 2 | + 帧率上限 60（把抖动变成恒定 60 fps） | 同上 |
| 3 | + 关闭焦散与 Beer-Lambert 吸收 | 同上 |
| 4 | + 关闭 RGB 色散 | 同上 |

降档判据：1 秒窗口内「**我们自己的工作**（渲染前准备 + Pass A + Pass B + Commit）
> 1.25 × 目标帧间隔」的渲染帧占比 >25%（电池 15%）连续 2 次。
**必须排除采集等待与 Present 阻塞**：那两段实测 8.7~25 ms，而我们自己的工作只有
30~80 µs/帧（8.33 ms 预算的 1/100）；旧判据用整循环时长，把等待算成丢帧，
结果单块拖拽也会被降到 level 2 —— 白白改视觉、fps 一点没变好。

升档需要连续 4 个干净窗口**且距上次降档 ≥20 s**；升上去 10 秒内被打回视为失败探针，
探针间隔翻倍（上限 300 s），避免在两个档位之间周期振荡。

`tools/verify_all.py` 的 7 项：跨版本 overlay / composed / capture 三项逐位对比
（需要先把 `82a49e8` 拉成 worktree 并打同一份合成采集源补丁，见 `performance_report.md` 第七节）、
脏矩形 vs 全量、跨屏拖拽、环境色 EMA 开关等价、DDA 局部拷贝 vs 整屏参考拷贝。

## 项目结构

```
liquid-glass/
├─ Cargo.toml / build.rs
├─ shaders/
│  ├─ glass_vs.hlsl         # 逐块矩形 -> 裁剪空间；SV_Position 为画布像素坐标
│  ├─ glass_ps.hlsl         # SDF + Snell 折射 + 色散 + Fresnel + 焦散 + 鼠标光
│  ├─ blur_down.hlsl        # 4-tap 角点下采样
│  ├─ blur_up.hlsl          # 8-tap tent 上采样
│  ├─ env_ema.hlsl          # 1/16 环境色探针时域平滑（方案 12.6 / R-20）
│  └─ reduce_env.hlsl       # 备用：独立 pass 版环境色探针（未接入管线）
├─ src/
│  ├─ main.rs               # 多屏画布管理、消息循环、事件驱动调度、脏区门控、
│  │                        #   独占全屏检测、自适应质量控制器
│  ├─ capture/mod.rs        # Capture 统一封装 + pick_backend()（方案 10.5）+ 合成采集源
│  ├─ capture/dda.rs        # 每屏一个 duplication + 脏矩形 + 局部拷贝 + 失效退避重建
│  ├─ capture/wgc.rs        # WGC 后端：CreateFreeThreaded + 帧生命周期 + 事件通知
│  ├─ gfx/
│  │  ├─ monitors.rs        # 显示器枚举（物理几何 + 每屏 DPI）
│  │  ├─ device.rs          # D3D11 设备 / DXGI 工厂 / 帧队列深度
│  │  ├─ shaders.rs         # HLSL 编译与状态对象
│  │  ├─ pyramid.rs         # Pass A：可裁剪的共享模糊金字塔 + 环境色 EMA
│  │  ├─ glass_renderer.rs  # Pass B：逐块 scissor 绘制
│  │  ├─ compositor.rs      # 画布、脏区推导、分级重绘、取证
│  │  ├─ gpu_timer.rs       # D3D11 timestamp（方案 7.6）
│  │  ├─ dcomp.rs / swapchain.rs
│  ├─ glass/                # 参数（含 DPI 缩放）、SDF/LUT、块模型、管理器
│  ├─ input/                # 光标轮询、拖拽状态机（逻辑单位，跨屏安全）
│  └─ ui/                   # 中文控制面板（内嵌 HTTP）
└─ tools/                   # 性能探针与逐像素自证脚本
```

## 实测（本机 3120×2080@120 Hz/200% + 1920×1080@100%，桌面持续重绘 47~110 次/秒）

| 指标 | 改造前（单屏） | 改造后（单屏折算） |
|---|---|---|
| Pass A / 帧 | 6.49 Mpx，每帧必跑 | **0.79 Mpx**，最低 **0 次/帧** |
| Pass B 脏区 / 帧 | 6.49 Mpx（整屏） | **0.50 Mpx**（单块）/ 1.5~2.0 Mpx（20 块） |
| DDA 拷贝 / 帧 | 6.49 Mpx（整屏） | 1.7~4.0 Mpx（脏矩形） |
| CPU render 均值 | 0.294 ms | **0.094~0.118 ms** |
| CPU render 峰值 | 17~31 ms | **0.66~0.94 ms** |
| 常驻 Private | 182.7 MB | **128.4 MB** |
| 20 块静止 CPU | 8.83% 单核 | **5.32%** |
| 20 块拖拽 CPU | 12.34% 单核 | **3.57~10.02%** |
| 锁屏 / 采集失效 | **83.25%**（96 kHz 空转） | **0.469%** |

视觉零变化取证：Pass A 重构前后 overlay **0 / 6,489,600 像素差异**；
脏矩形路径 vs 全量路径 **6/6 场景 0 差异**；局部拷贝 vs 整屏参考拷贝 **7 轮 840 帧 0 差异**；
多屏改造前后主屏 overlay **alpha 通道 0 差异**。详见 `performance_report.md`。
