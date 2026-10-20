# 渲染管线契约（逐帧）

## 1. Scope / Trigger

任何改动 `src/gfx/**`、`src/capture/**`、`shaders/**`、`src/main.rs` 主循环的任务，
必须先读本文与 `graphics-d3d11-gotchas.md`；改完必须跑 `BASELINE.md` 的全部阀门。

## 2. 逐帧顺序（单渲染线程，方案 10.4）

```
MsgWait/PeekMessage 泵
→ 每屏采集 try_capture(wait)          # DDA: AcquireNextFrame(阻塞即主循环休眠点) + 脏矩形 + 局部拷贝
                                       # WGC: 帧池取帧；HDR 输出自动选 WGC（见 §4）
→ prep：面板参数同步、质量档位 apply、块几何/投影、脏区计算
→ 整屏 ClearRenderTargetView(0,0,0,0)  # 每帧自包含，见不变量 1
→ Pass A（仅当 run_rect 有值）：下采样×4 → 环境色 EMA(门控) → frost 链(4 tent) → 上采样；全部 scissor
→ Pass B：z 序画**全部块**，per-block viewport + scissor = 块矩形⊕shadow_margin
→ [取证帧?] → Present(1) → DComp Commit（每屏一次）
```

触发源全空则整帧跳过（不 Present、不 Commit）= 事件驱动终点。

## 3. 契约（签名 / 常量 / 环境变量）

- 影响盒半径单一来源：`glass::params::cursor_influence_px(r) = (r*3.0+32.0) as i32`；
  `src/gfx/compositor.rs` 与 `src/main.rs` 都必须用它；**必须严格大于** `glass_ps.hlsl` 里
  gate 的支撑半径 `3*mouse_radius`（单测 `influence_box_strictly_covers_shader_gate_support` 盯着）。
- 丢帧判据单一入口：`QualityCtrl::is_miss(own_us, target_us) = own_us > target + target/4`；
  传入的必须是 **own_us = prep + PassA + PassB + Commit**（`last_stage_us[0]+[1]+[3]`），
  **禁止**传 loop 总时长或 render_us（含 Present 阻塞）。
- 脏区三来源**始终并集**（不得短路）：桌面脏区⊕(BLUR_REACH+pad) 命中的块整块、
  位移块旧∪新矩形、光标影响盒命中的块整块。
- 金字塔有效性账本 `pyramid_valid`：只允许声称**本帧真正重算过**或**之前已声称且未被桌面变化污染**的区域；
  `!covered` 分支必须排在 `backdrop_changed` 分支之前。
- cbuffer 语义（`glass_ps.hlsl`）：c11=鼠标(xy)/半径/强度，c12=spread/hover/press/proximityGlow，
  c13=拖拽速度(xy)/lag（**仅被拖块非零**），c16=块原点/env/LOD3。鼠标对**非 hover 块**的影响
  必须乘 `fall × gate(2.5R..3R)`，盒外精确为 0。
- 环境变量（完整表见 README）：`GLASS_METRICS / GLASS_MANY / GLASS_SPREAD / GLASS_FAR_GRID /
  GLASS_SYNTH_CAP / GLASS_SIM_BACKDROP / GLASS_SIM_BUSY / GLASS_BACKEND / GLASS_MONITORS /
  GLASS_ADAPTIVE / GLASS_FROST_HALF / GLASS_FRAME_LATENCY / GLASS_INTERNAL_SHOT(+_AT_MS) /
  GLASS_SHOT_EVERY_MS / GLASS_VERIFY_PASS_A / GLASS_VERIFY_COPY / GLASS_VERIFY_VALIDITY /
  GLASS_FORCE_FULL / GLASS_SKIP_{COPY,DIRTY,PASS_A,PASS_B,CLEAR,PRESENT} / GLASS_EXTREME_SPEC`。

## 4. 采集后端选择（HDR 规则）

`IDXGIOutput6::GetDesc1().ColorSpace != RGB_FULL_G22_NONE_P709` → **默认 WGC**（DWM 替我们做
HDR→SDR）；否则 DDA 优先、失败回退 WGC。`GLASS_BACKEND` 可强制；强制 dda 且拿到 FP16 时打
`[warn]`（背景会过曝）。**禁止**使用 windows 0.39 的 `DuplicateOutput1`（绑定与 COM 签名不符）。

## 5. Validation & Error Matrix

| 情况 | 行为 |
|---|---|
| DDA ACCESS_LOST（锁屏/被抢占） | 隐藏该屏画布、500ms 退避重建、`[warn]` 只打一次 |
| WGC 不可用（HDR 屏） | 回退 DDA + `[warn]`（背景可能过曝） |
| 独占全屏应用 | 该屏画布隐藏，退出后恢复（R-16） |
| 空屏（无块）1s | 画布隐藏（混合刷新率保护）；采集降 4Hz 保活 |
| 金字塔/RT 重建 | `invalidate()`：pyramid_valid 清空、下一帧整屏 Pass A |

## 6. Good / Base / Bad

- Good：每帧自包含（整屏清 + 画全部块）；Pass A 按脏区 scissor；取证用连拍看时间维度。
- Base：脏区并集只 union 整块矩形（不做像素级 region），换取 O(块数) 的书本账。
- Bad：依赖 back buffer 跨帧持久（flip 模型不保）；按 dirty 的外接 bbox 清屏或裁 Pass B；
  用 loop/render 时长判丢帧；给非 hover 块不加衰减的鼠标响应。

## 7. Wrong vs Correct

| Wrong | Correct |
|---|---|
| `ClearRenderTargetView` + 期望 scissor 生效 | 整屏清 + 画全部块（或带 scissor 的零填充绘制） |
| `quality.tick(loop_us, …)` | `quality.tick(prep+A+B+C, …)` |
| `pyramid_valid = n`（只刷了脏区∩n 却声称 n） | 只声称重算过的区域；`!covered` 先于 `backdrop_changed` |
| 非 hover 块 `normalizedMouse` 不衰减 | `nMouse = normalizedMouse * max(fall*gate, hover)` |
| HDR 屏用 DDA 的 8bit/FP16 复制面 | HDR 输出自动切 WGC |

## 8. Tests Required

`verify_dirty`（增量==全量）、`repro_tremble`（时间维度稳定）、`verify_mouse_isolation`（跨块隔离）、
`verify_validity`（虚报/漏画计数）、`verify_adaptive`（闭环）、`verify_crossscreen`（多屏 DPI）、
`gate_perf`（吞吐/内存硬门槛）、`cargo test`（25 单测）。期望值见 `measurement-and-gates.md`。
