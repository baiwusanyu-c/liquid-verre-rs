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

## 性能指标清单与质量阀门（重构的回归安全网）

指标（方案第 1 章 + M2/M3）：① 块数 1–200 动态增删；② 尺寸 64×64–整屏；③ 20 块常驻显存 <200 MB；
④ 拖拽 ≥60 fps（120 Hz 屏 ≥90 fps）；⑤ 拖拽 CPU <5% 单核（M2：≤1%）；静止 0 帧 0 CPU；
无残影/延迟感；多屏 + DPI；M2 ② 交互式 resize 无 stall；M3 24h 内存漂移 <2 MB。

阀门命令（重构前后都必须全绿）：

```powershell
cargo test --release              # 单元测试 25 个：矩形运算 / DDA 元数据 sanitize / 丢帧判据 1.25x 阈值 /
                                  # 参数夹紧 / DPI 缩放 / 影响盒与 3σ 支撑对齐 / LUT 单调 / 拖拽 EMA / 块 clamp
python tools/gate_perf.py         # 性能阀门：硬门单块 fps≥90 且 p50≤9ms、20 块默认≥60、
                                  # 20 块 lat=2 ≥90、Private<200MB；软门 CPU<5% 与 20 块默认≥90
```

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


---

# 重构后状态（0.4.0 / liquid-verre-rs，2026-10-20）

上面全部内容仍是**重构前基线快照**，保留不动（它是逐位比对的参照系）。
本节记录重构落地后的口径变化与验收结果；细节过程见
`.trellis/tasks/10-20-lib-refactor-liquid-verre/` 与 `CHANGELOG.md`。

## 命令口径变化

| 旧 | 新 |
|---|---|
| `cargo build --release` → `target/release/liquid-glass.exe` | `cargo build --release --workspace` → `target/release/liquid-verre-playground.exe`（dev playground；库本体是 `libliquid_verre.rlib`） |
| `cargo test --release`（25 个） | `cargo test --release --workspace`（**147 个**：lib 129 + 集成 11 + playground 5 + doctest 2） |
| — | `cargo clippy --workspace --all-targets --release -- -D warnings`（0 warning） |
| — | `cargo fmt --all --check` |
| — | `node scripts/check.mjs` / `build.mjs` / `package.mjs` / `gate.mjs`（等价于 `cargo xtask <cmd>`；脚本已从 PowerShell 迁到 Node） |
| 8 个回归脚本 | 同 8 个，但被测 EXE 变为 playground 产物；**必须带 `GLASS_DEFAULT_BLOCK=1`**（脚本已内置）—— 库默认不再自建窗口 |

## 视觉验收（tools/cmp_visual.py，新口径）

- 口径：稳态帧（`GLASS_INTERNAL_SHOT_AT_MS=2500`）+ 光标 `SetCursorPos` 物理停放（看门狗持续复位）
  + 停放点离玻璃 > 3*mouse_radius；四段 oracle = deep / interior / ring / outside。
- **single 跨版本（重构前 exe vs 重构后）：四段全 0 差异 = 逐位一致。**
- many 同一构建两次：全 0（确定性）。
- many 跨版本：差异 100% 归因到圆角半径的**有意变更**（圆角邻域 r+6 内 36729 px / 最大 136；
  邻域外 3105 px / 最大 2 = 阴影尾部残差，≤ 阈值 2）。原因：旧主循环圆角滑杆只写「选中块」，
  20 块里 19 块的 radius 永远停在 `Block::new` 的硬编码 28；新库统一由样式派生（需求 F1）。
- 光标在深内部两点移动：deep 段 0 差异（背景不颤抖）。

## 回归阀门结果（2026-10-20）

| 脚本 | 结果 |
|---|---|
| verify_dirty ×3 | PASS |
| verify_mouse_isolation | PASS |
| verify_validity | PASS |
| verify_crossscreen | PASS |
| verify_env_ema ×3 | PASS（补 `GLASS_ADAPTIVE=0` 后） |
| verify_adaptive | **环境阻塞**：当前机器 `rel_max ≈ 200~257 ms`（DDA `ReleaseFrame` 阻塞），远超健康窗口 `rel_max<5ms`；旧二进制同条件同样 FAIL，背靠背 A/B 无差别 → 非回归 |
| gate_perf | 同上，环境阻塞；`own_us`（我们自己的工作）40~84 µs，与基线同量级 |

> 性能结论只在健康窗口给出（见 `.trellis/spec/backend/measurement-and-gates.md`）。
> **待办：在 `rel_max<5ms 且 acq_max<25ms` 的窗口复跑 gate_perf 与 verify_adaptive 并回填本节。**

## 不变量增补（在原 9 条之上）

10. **缩放反馈掩码源头互斥**：`ResizeEdges::to_shader_edges()` 在对角时全 0、
    `to_shader_corners()` 在单边时全 0；border 用独立着色器 `border_ps.hlsl` + 独立 `BorderCb`，
    但**在第 i 块玻璃之后、任何更高块之前**发射（0.5.2 #1 起随 `draw_blocks` 逐块内联，
    不再是「全部玻璃之后的独立叠加 pass」）；判据本体 `core::draw::border_emit_plan`，
    只作用于 hover 块，无 feedback 时零状态切换。取证 `tools/verify_border_zorder.py`。
11. **影响盒单一出口**：`MonitorRt::influence_px() = max(3R+32, 2*grip+16) + ceil(shadow_margin)`，
    脏区推导 / verify_validity / mouse_relevant 三处共用，禁止手写外扩。
12. **默认参数即视觉基线**：`LiquidParams::default()` 的 `blur_min` 必须写成表达式 `20.0 * 0.12`
    （f32 下 = 2.3999999，字面量 2.4 差 1 ULP ≈ 2400 个 ±1 LSB 像素）；改默认值 = 改基线，
    必须同步取证图与本节。
13. **绘制的层序与命中的层序必须共用同一套 z 判据**（0.5.3 新增，#9 的直接教训）：
    「谁在上面」这件事在三处必须同口径 —— 绘制
    （`core::draw::border_emit_plan`：border 在第 i 块玻璃之后、任何更高块之前）、
    本体命中（`core::manager::hit_test`：`max_by_key(z_order)`）、
    把手命中（`core::manager::hit_resize`：先 `topmost_body_z_at` 过滤再比 z）。
    三者的矩形口径一律用**精确本体**（`x..x+width`，不含阴影外扩、不含 grip 外扩）。
    ⚠ 只修其中一处 = 半修：0.5.2 #1 修对了绘制层序（白边不再盖到上层框身上），却没修命中判据，
    于是「下层框被压住的那条边缘照样亮、手型照样变缩放箭头、点下去没反应」——
    宿主复验直接判失败。改任一处前先问「另外两处同不同意」。
