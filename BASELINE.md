# 重构基线（baseline/pre-refactor）

本文件是重构前的**基线快照说明**。基线 commit 见 `git rev-parse baseline/pre-refactor`；
备份分支 `backup/pre-refactor-baseline` 指向同一 commit；过程证据与完整历史在
`performance_report.md`（第一~十四轮）。

## 构建 / 运行

```powershell
cargo build --release          # 产物 target/release/liquid-glass.exe
cargo run                      # debug 亦可（元数据 sanitize 后 debug 不再 panic）
```

环境变量速查（完整表见 README）：`GLASS_METRICS=1`、`GLASS_MANY=1`、`GLASS_SYNTH_CAP=1`、
`GLASS_SIM_BACKDROP=WxH@HZ`、`GLASS_ADAPTIVE=0`、`GLASS_FRAME_LATENCY=1|2`、
`GLASS_BACKEND=dda|wgc`、`GLASS_INTERNAL_SHOT=<path>`、`GLASS_SHOT_EVERY_MS=<n>`、
`GLASS_VERIFY_VALIDITY=1`、`GLASS_SIM_BUSY=<ms>`、`GLASS_FAR_GRID=1`、`GLASS_CURSOR_AT=x,y`。

## 基线指标（3120×2080@120Hz/DPI192 + 1920×1080@60Hz/DPI96；Intel Ultra 9 285H / Arc 140T）

| 项 | 默认（lat=1，跟手优先） | 吞吐档（GLASS_FRAME_LATENCY=2） |
|---|---|---|
| 单块拖拽 fps / p50 | 113~116 / 8.32~8.36 ms（120Hz 锁相） | 113.9 / 8.35 ms |
| 20 块拖拽 fps / p50 | 70~74 / 13.0~13.2 ms | **105.0 / 8.34 ms** |
| 拖拽 CPU 单核 | 安静桌面 1.7~3.6%；高重绘桌面 8.9~11.9% | 同量级 |
| 20 块常驻 Private | 158.9~160.2 MB（双屏） | 同 |
| 输入滞后 | 最小 | +1 帧呈现队列 ≈ +8 ms |

指标口径与未达标项（20 块 ≥90 仅在吞吐档达成；CPU <5% 在高重绘桌面窗口超标）见
`performance_report.md` 第十三/十四轮。

## 回归命令与期望（重构后必须全绿）

```powershell
python tools/verify_dirty.py            # 4/4 PASS，0 差异（增量 == 全量）
python tools/verify_mouse_isolation.py  # [1][2][3] PASS（拖一块其余块 0 像素变化）
python tools/verify_validity.py         # 0 虚报 / 0 漏画
python tools/repro_tremble.py far       # PASS（静态桌面拖拽，见证块相邻帧 0 差异）
python tools/repro_tremble.py many      # PASS（密集 20 块同上）
python tools/verify_adaptive.py         # 降档/回升闭环 PASS
python tools/ab_latency.py              # latency 1 vs 2 轮转 A/B（吞吐档依据）
python tools/ab_memory.py               # 常驻内存受控 A/B（level 0 vs level 1）
```

## 基线包含的关键不变量（重构时不要破坏）

1. **每帧自包含**：flip 模型 back buffer 在 Present 后内容不保 → 每帧整屏清 + Pass B 画全部块；
   Pass A 仍按脏区 scissor（成本大头）。`ClearRenderTargetView` 忽略 scissor，别再回到"只清脏区"。
2. **金字塔有效性不虚报**：只声称真正重算过的区域；`!covered` 分支必须排在 `backdrop_changed` 前。
3. **模糊链幂等**：下采样外扩 14/12/8/0、tent 链 32/24/16/8、首级外接框 ⊕48；局部重算必须等于全量重算。
4. **环境色 EMA 只在捕获内容真变化时推进**（否则拖拽时全体块 ±1~2 LSB 色调跳变）。
5. **鼠标影响 3σ 有限支撑**：border 渐变乘 `fall × gate(3R)`，影响盒半径 3R+32 与之严格对齐。
6. **脏区三来源始终并集**：桌面脏区、位移、光标影响盒（不能被 `if dirty.is_none()` 短路）。
7. **HDR 输出自动切 WGC**（DDA 复制面是 HDR 参考值，会整张过曝）；SDR 输出 DDA 优先。
8. **自适应控制器判据只看"我们自己的工作"**（prep+PassA+PassB+Commit），排除采集等待与 Present 阻塞。
9. **元数据 sanitize**：DDA 脏/移动矩形是不可信外部输入，先 clamp 到屏幕并丢弃退化矩形。

## 已知偏离方案（重构时可逐条决策）

12.1 双模渲染器/W 模式、12.2 输入壳窗口、12.5 瓦片级检测、7.4-3 capture_scale、
5.10 累积缓冲、M3 崩溃恢复/开机自启/24h 浸泡、M4 可选项、7.6 PresentMon 口径、
12.4.1 Pass B 分级重绘（因不变量 1 主动放弃）、7.4-7 队列深度（默认 1 为跟手，2 为吞吐档）。
