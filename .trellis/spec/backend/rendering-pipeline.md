# 渲染管线契约（逐帧）

## 1. Scope / Trigger

任何改动 `crates/liquid-verre/src/gfx/**`、`src/capture/**`、`src/runtime/**`、
`src/core/{geometry,draw,handle,quality,sdf}.rs`、`shaders/**` 的任务，
必须先读本文与 `graphics-d3d11-gotchas.md`；改完必须跑 `measurement-and-gates.md §5` 的全套阀门。

## 2. 逐帧顺序（单渲染线程，方案 10.4，**不可随意调换**）

```
PeekMessage 泵（PM_REMOVE；WM_QUIT → app.shutdown）
→ 每屏采集 try_capture(wait)          # DDA: AcquireNextFrame 阻塞 = 主循环唯一休眠点 + 脏矩形 + 局部拷贝
                                      # WGC: 帧池取帧；HDR 输出自动选 WGC（见 §4）
                                      # wait: 刚激活/本屏不需要背景 = 32 ms，否则 capture_wait_ms
→ ★ 帧首一次性消费上层命令            # runtime/commands.rs::apply(&mut app)：drain Command 队列
                                      #   一帧内连发 N 条同类 setter 只有最后一条生效（输入合并，方案 12.2.3）
→ prep：默认参数(default_style.to_params) + 质量档 apply、块几何/跨屏投影(StyleCache)、
        鼠标状态(read_cursor，钉死光标优先)、hover + 把手判定(先 hit_resize 再 hit_test)、
        拖拽/缩放每帧应用一次、每屏 collect_monitor_blocks、窗口区域(SetWindowRgn)、
        点击穿透、光标影响盒 → mouse_relevant、独占全屏 4 Hz 轮询、快照发布(dirty 即时 / 4 Hz 兜底)
→ 渲染门控：触发源全空则整帧跳过（不 Clear、不 Pass A/B、不 Present、不 Commit）
→ 每屏 render_monitor：
     整屏 ClearRenderTargetView(0,0,0,0)      # 每帧自包含，见不变量 1
     → Pass A（仅当 needs_backdrop && run_rect 有值 && 未被限频）
     → Pass B（z 序画**全部块**，per-block viewport + scissor）
     → [取证帧?] → Present(1) → DComp Commit（每屏一次）
→ QualityCtrl.tick(own_us, presented)   # own_us = prep + sim + Σ(A+B+C)，**不含**采集等待与 Present 阻塞
→ report_metrics（GLASS_METRICS=1 时，5 秒墙钟窗口一行 [metrics]）
```

**全透明屏跳过 Pass A**（需求 F3.1 的性能来源）：`MonitorRt::needs_backdrop` = 本屏是否存在
`GlassMode::Liquid` 的可见块；为 `false` 时 `FrameInput::needs_backdrop = false`，
`Compositor::render` 走 `!needs_backdrop` 分支——整条 Pass A 跳过、`last_stage_us[0]=0`、
且**不动** `background_valid` / `pyramid_valid`（透明屏既没重算也没污染，账本保持原样最诚实）。
同一标志还让 `wait_for_captures` 跳过该屏（不等采集就绪，启动更快），
并让主循环的 `render_monitor` 前置条件 `!ready && needs_backdrop` 不拦它。

Pass A 内部顺序（`gfx/pyramid.rs::run`，全部按 `region` scissor）：

```
下采样 capture → down[0]=1/2 → down[1]=1/4 → down[2]=1/8 → down[3]=1/16
   scissor 逐级外扩 DOWN_GROW = [14, 12, 8, 0]（幂等性要求，见不变量 3）
→ 环境色 EMA（仅 env_advance = 「捕获内容真变化」时推进；region = 块足迹）
→ frost 链：capture → frost_half[0]（1/2 分辨率）→ 4 次 tent ping-pong（外扩 g = 32 - 8*pass）
   → 非半分辨率档时上采样回整屏 frost（soft 与 frost 共享同一份 RT/SRV）
→ 收尾：PSSetShaderResources(t0..t8) 全部解绑（Pass B 会重绑这九个槽）
```

已移除的三处纯浪费（输出逐像素一致）：`up` 上采样链（`SamplePyramid` 从未被 `ps_main` 调用）、
`soft` 独立 RT、`env16` 拷贝（直接复用 `down[3]`）。

## 3. 契约（签名 / 常量 / cbuffer / 环境变量）

### 3.1 影响盒：单一来源

```
core::geometry::cursor_influence_px(mouse_radius) = (mouse_radius * 3.0 + 32.0) as i32
core::geometry::resize_influence_px(grip_px)      = (grip_px.max(0.0) * 2.0 + 16.0) as i32
core::geometry::frame_influence_px(R, grip)       = max(cursor_influence_px(R), resize_influence_px(grip))
runtime::monitor::MonitorRt::influence_px()       = frame_influence_px(self.mouse_radius, self.max_grip.max(1.0))
                                                    + self.margin.ceil().max(0.0) as i32
```

即 **影响盒 = `max(3R+32, 2*grip+16) + ceil(shadow_margin)`**。
`MonitorRt::influence_px()` 是**唯一出口**，消费者：`gfx/compositor.rs` 的脏区推导与
`verify_validity` 自检、`runtime/mod.rs` 的 `mouse_relevant` 判定。任何一处手写外扩都是回归。
单测 `influence_boxes_strictly_cover_shader_support` 钉住「严格大于着色器支撑半径」。

窗口区域外扩是另一个量，不要混：`MonitorRt::region_margin() = max(margin, max_grip + 2.0)`
（缩放命中带必须伸出块外才能从块外抓住边缘，`SetWindowRgn` 也要跟着外扩，
否则那部分鼠标消息根本到不了我们窗口）。

### 3.2 丢帧判据：单一入口

`QualityCtrl::is_miss(own_us, target_us) = own_us > target + target/4`。
传入的必须是 **own_us = prep + Σ(PassA + PassB + Commit)**（`last_stage_us[0]+[1]+[3]`），
**禁止**传 loop 总时长或 `render_us`（两者都含 Present 阻塞）。

### 3.3 脏区三来源**始终并集**（不得短路）

1. 桌面脏区 ⊕ (`BLUR_REACH=96` + `pad`) 命中的块 → 整块矩形；
2. 位移块 → 旧矩形 ∪ 新矩形；
3. 光标影响盒（用上一帧与本帧光标连线扫过的盒）命中的块 → 整块矩形。

`pad = (128.0 + (vel * max_lag_seconds).min(128.0)) as i32`；`HYSTERESIS = 192`
（桌面没变、只有块移动时的足迹外扩迟滞）。

金字塔有效性账本 `pyramid_valid`：只允许声称**本帧真正重算过**或**之前已声称且未被桌面变化污染**
的区域；`!covered` 分支必须排在 `backdrop_changed` 分支之前。

### 3.4 cbuffer 语义表（`glass_ps.hlsl` 的 `cbuffer GlassParams : register(b0)`）

槽位数 `CB_FLOATS = 19 * 4`（`gfx/glass_renderer.rs`）。**`c0..c17` 的语义一字未改** ——
它们是历史取证图的比对基准，改一个字段就等于作废全部逐位对比。新增语义一律往后加。

| 槽 | 内容 | 备注 |
|---|---|---|
| `c0` | 渲染目标原点(画布px) x,y / 块窗口尺寸 w,h | `local = canvasPx - c16.xy - c0.zw*0.5` |
| `c1` | 显示器原点 x,y / 显示器尺寸 w,h | |
| `c2` | 时间 t / dpiScale / aaWidth / shadowMargin | dpiScale 恒 1.0，aaWidth 恒 1.25 |
| `c3` | superN / cornerRadius / lensCoverage / bevelWidthPx | `Shape()` 用 `c3.y`(圆角) 或 `c3.x`(squircle 指数) |
| `c4` | ior / thickness / refrStrength / dispersion | |
| `c5` | clarity / blurMax / blurMin / progressiveCurve | blurMax 直接进金字塔 LOD |
| `c6` | tintOpacity / adaptiveTint / baseAlpha / shadowOpacity | `c6.w` 进 `GaussianTail` 阴影 |
| `c7` | tintColor rgba | 透明模式的颜色也走这里 |
| `c8` | 主光方向 xyz / lightIntensity | |
| `c9` | specIntensity / specPower / fresnelF0 / rimWidth | |
| `c10` | specSpread / edgeAlways / causticGain / absorption | |
| `c11` | 鼠标(屏幕px) x,y / mouseRadius / mouseIntensity | |
| `c12` | mouseSpread / hover / press / proximityGlow | `hover` 仅对 hover 块非 0；`press` 是全局值每块都写，但其作用被 `hoverGain`（含 `fall*noFb`）门住，与重构前逐位一致 |
| `c13` | dragVelocity x,y / lagSeconds / pad | **仅被拖块非零**（否则其它块背景会跟着晃） |
| `c14` | mouseColor xyz / envTintMix | |
| `c15` | shapeMode / distScale / maxSlope(=3.0) / pad | shapeMode: 0=RoundedRect, 1=Squircle |
| `c16` | 块窗口左上角(画布px) x,y / envEma(第 0 通道) / lod3 | |
| `c17` | 画布屏幕原点 x,y / 画布尺寸 w,h | 与 `c1` 同值（历史保留，勿删） |
| **`c18`** | **(mode, transparentOpacity, handleGrabbed, -)** | ★新增。`mode > 0.5` ⇒ 透明玻璃早退分支；`transparentOpacity = base_alpha`（已含颜色 alpha）；`handleGrabbed = 1` ⇒ 缩放把手被抓住 |

`handleGrabbed` 的作用：液态着色器里原有的 hover 高光 / 鼠标跟随渐变
（`glint` / `hoverGain` / `prox` / `mouseField`）乘 `noFb = 1 - handleGrabbed` 让位给
border pass，否则悬停把手时直边会残留一条白色高光（参考图要求对角悬停时直边干净）。
**未抓住时 `handleGrabbed = 0` ⇒ `hv == c12.y`，液态分支与重构前逐位一致。**

鼠标对**非 hover 块**的影响必须乘 `fall × gate(2.5R..3R)`，盒外精确为 0。

### 3.4b 缩放反馈 border = 独立叠加 pass（**不在** `glass_ps` 里）

`shaders/border_vs.hlsl` + `shaders/border_ps.hlsl`，自有 `cbuffer BorderCb : register(b0)`
（`b0` 绘制矩形、`b1` 半尺寸/圆角/AA、`b2` 直边掩码、`b3` 对角掩码、`b4` 宽度/不透明度、
`b5` 颜色、`b6` 画布尺寸），由 `GlassRenderer::draw_borders()` 在 **Pass B 之后**绘制，
共用 premultiplied blend 与 scissor 光栅化状态，因此叠加语义与玻璃自绘的白色一致。

- 只画 `DrawBlock::feedback` 非零的块（通常 0~1 块）；全零时 `draw_borders` 直接返回，
  **零状态切换**（这是「拖一块、其余块逐位不变」的前提之一）。
- 直边 mask = band × 「圆角切点窗」→ 到切点为止、**不拐弯**（需求 F2.1）；
  对角 mask = band(SDF 内侧距离) × 「圆角象限窗」→ **沿弧拐弯**（需求 F2.2）。
- 两种掩码在**源头互斥**：`ResizeEdges::to_shader_edges()` 在对角时返回全 0、
  `to_shader_corners()` 在单边时返回全 0（`core/handle.rs` 有单测钉住）。
  若把 `TOP_LEFT` 打成 `left | top` 送进 `b2`，切点附近会叠出亮斑。
- 输出乘 `coverage`（SDF 覆盖），否则 border 会漏到玻璃形状外变成白色实心矩形。
- premultiplied source-over 的衰减因子是 **(1 - 下层 alpha 之外的) border 自身 alpha**：
  `A = Ab + Aa*(1-Ab)`、`C = Cb + Ca*(1-Ab)`。用错因子会「只加不透明度不加颜色」，
  玻璃内部 alpha≈1 时 border 处反而变暗（重构过程中实测抓到过：暗 120 级）。
- **默认值（0.4.1 起）**：`ResizeFeedbackOptions::default()` = `border_width 5.0` 逻辑像素、
  `alpha 1.0`、纯白 `[1,1,1]`；投影时 `DrawBlock::feedback.width = border_width * scale`
  （200% DPI ⇒ 10 物理像素）。此前是 `3.0 / 0.95`：在 100% DPI 屏上实测「看得见但不醒目」，
  用户验收时反馈触发不够明显。改这两个默认值 = 改**默认视觉**，但：
  ① 不触碰 `c0..c18`（宽度 / 不透明度 / 颜色全在独立的 `BorderCb.b4/b5` 里）；
  ② **不需要重算影响盒** —— band 用的是 SDF **内侧**距离
  （`inside = max(-sd, 0)`、`aaBand = min(aa, bw * 0.5)`、`band = 1 - smoothstep(bw - aaBand, bw, inside)`，
  即实芯 + 外沿 AA；历史写法 `1 - smoothstep(0, bw, inside)` 从第 1 像素渐隐、实测只读得到约 2 px），画在玻璃形状内部，
  支撑窗没有变宽，因此 `resize_influence_px` 仍只由 `grip` 决定（不变量 5 不受影响）；
  ③ 跨版本像素取证也看不到它：`cmp_visual.py` 把光标停放在离玻璃 > `3 * mouse_radius` 处，
  hover 与把手都不命中 ⇒ `feedback == ResizeFeedback::NONE`，`draw_borders` 直接返回。
  要验这次改动只能用连拍（`GLASS_SHOT_EVERY_MS`）+ 肉眼，或把光标物理停放到把手上。

### 3.5 环境变量

完整表见 `README.md`。主循环那批（`GLASS_METRICS` / `GLASS_DEFAULT_BLOCK` / `GLASS_MANY` /
`GLASS_SPREAD` / `GLASS_FAR_GRID` / `GLASS_EXTREME_SPEC` / `GLASS_SIM_BUSY` / `GLASS_SIM_BACKDROP` /
`GLASS_MONITORS` / `GLASS_VERIFY_PASS_A` / `GLASS_INTERNAL_SHOT` / `GLASS_CURSOR_AT` / `GLASS_ADAPTIVE`）
由 `diag/env.rs::EnvOverrides::load()` 在**启动期解析一次**；子系统那批（`GLASS_BACKEND` /
`GLASS_SYNTH_CAP` / `GLASS_COPY_MODE` / `GLASS_SIM_DIRTY` / `GLASS_SKIP_*` / `GLASS_VERIFY_COPY` /
`GLASS_ENV_EMA` / `GLASS_FROST_HALF` / `GLASS_PASS_A_HZ` / `GLASS_SHOT_EVERY_MS` / `GLASS_FORCE_FULL` /
`GLASS_VERIFY_VALIDITY` / `GLASS_INTERNAL_SHOT_AT_MS` / `GLASS_GPU_TIMINGS` / `GLASS_BURST_OVERLAY` /
`GLASS_FRAME_LATENCY` / `GLASS_WGC_BUFFERS`）只在自己模块的**构造期**读一次。
两者都禁止进热路径（每帧 `env::var` = 一次进程环境锁 + 一次 `String` 分配）。

`GLASS_CURSOR_AT=x,y` 现在是**全程钉死**（`App::pinned_cursor` → `App::read_cursor()`），
不再只设初值：窗口过程与主循环都必须走 `read_cursor()`，否则命中测试用真实光标、
渲染用钉死光标，两者会打架。

## 4. 采集后端选择（HDR 规则）

`IDXGIOutput6::GetDesc1().ColorSpace != RGB_FULL_G22_NONE_P709` → **默认 WGC**（DWM 替我们做
HDR→SDR；实测 WGC 请求 B8G8R8A8 时与 SDR 桌面统计一致）；否则 DDA 优先、失败回退 WGC。
`GLASS_BACKEND=dda|wgc` 可强制；强制 dda 且拿到 `R16G16B16A16_FLOAT` 复制面时打一次 `[warn]`
（背景会过曝，warn-once 由 `hdr_warned` 保证）。
**禁止**使用 windows 0.39 的 `DuplicateOutput1`（绑定与 COM 签名不符）；本项目锁 `windows = "0.58"`。

## 5. 九条不可破坏的不变量

1. **每帧自包含**：flip 模型 back buffer 在 Present 后内容不保 → 每帧整屏清 + Pass B 画全部块；
   Pass A 仍按脏区 scissor（成本大头）。`ClearRenderTargetView` **忽略 scissor**，别再回到「只清脏区」。
2. **金字塔有效性不虚报**：只声称真正重算过的区域；`!covered` 分支必须排在 `backdrop_changed` 前。
   全透明屏既不重算也不声称（账本原样保留）。
3. **模糊链幂等**：下采样外扩 `DOWN_GROW = [14,12,8,0]`、tent 链 `g = 32 - 8*pass`（32/24/16/8）、
   Pass A 采样余量 `pad = 128 + min(vel*max_lag, 128)`、`BLUR_REACH = 96`、`HYSTERESIS = 192`。
   局部重算必须等于全量重算（`verify_dirty` 连跑 ≥3 次）。
4. **环境色 EMA 只在捕获内容真变化时推进**（`env_advance`）：否则拖拽时全体块 ±1~2 LSB 色调跳变。
5. **一切「光标驱动的视觉」都是有限支撑窗，且影响盒必须严格覆盖它。** 现在有两项，不是一项：
   - **鼠标点光**：border 渐变乘 `fall × gate(2.5R..3R)`，支撑半径 `3*mouse_radius`，
     由 `cursor_influence_px(R) = 3R + 32` 覆盖；
   - **缩放悬停反馈**（重构新增）：白色 border 只在光标落进 handle 命中带时出现，
     命中带宽 `grip_px * scale`，由 `resize_influence_px(grip) = 2*grip + 16` 覆盖。
   两者取大者再加 `ceil(shadow_margin)`（块的绘制矩形本身比几何矩形大一圈），
   即 `MonitorRt::influence_px()`。**盒外块的鼠标项与缩放反馈项都精确为 0**，
   盒内块整块重画，因此不会出现拼接缝。
   配套两条硬约束：
   - **直边 / 对角掩码在源头互斥**：`ResizeEdges::to_shader_edges()` 在对角时返回 `[0;4]`，
     `to_shader_corners()` 在单边时返回 `[0;4]`（单测 `shader_packing_matches_semantics` 钉住）。
     若两者同时非 0，着色器会同时画「到切点为止的直线」与「沿圆角的弧」，
     在切点附近叠出一个亮斑（需求 F2.1 明确不要）。
   - **反馈只作用于被 hover 的那一块**：其余块 `feedback = ResizeFeedback::NONE`
     → `draw_borders` 直接跳过该块（全零掩码时整个 pass 零状态切换）。
     这是「拖一块、其余块逐位不变」（`verify_mouse_isolation` / `repro_tremble`）仍然成立的前提。
   新增任何光标驱动效果时，**必须**先在 `core/geometry.rs` 里给出它的支撑半径并纳入
   `frame_influence_px`，再写着色器；顺序反了就是下一次拼接缝。
6. **脏区三来源始终并集**：桌面脏区、位移、光标影响盒（不能被 `if dirty.is_none()` 短路）。
7. **HDR 输出自动切 WGC**（DDA 复制面是 HDR 参考值，会整张过曝）；SDR 输出 DDA 优先。
8. **自适应控制器判据只看「我们自己的工作」**（prep+PassA+PassB+Commit），
   排除采集等待（实测 8~32 ms）与 Present 阻塞（实测 6.6~25 ms）。
   我们自己的工作只有 30~80 µs/帧（8.33 ms 预算的约 1/100）。
9. **元数据 sanitize**：DDA 脏/移动矩形是不可信外部输入，先 clamp 到屏幕并丢弃退化矩形
   （`sanitize_rect` 有单测）；一切来自上层的数值先过 `sanitize()` / `clamp_finite()` 才进 GPU
   （`f32::clamp` 对 NaN **原样返回 NaN**，NaN 进 GPU 会让 SDF 整块消失且不报错）。

## 6. Validation & Error Matrix

| 情况 | 行为 |
|---|---|
| DDA ACCESS_LOST（锁屏/被抢占/独占全屏） | 隐藏该屏画布、按 `retry_backoff_ms`（默认 500 ms）退避重建、`[warn]` 只打一次（`capture_error_logged`）、发 `EngineEvent::CaptureLost` |
| 采集恢复 | `[info]` + `EngineEvent::CaptureRestored`；`compositor.invalidate()`（金字塔与后备缓冲都失效，下一帧整屏重来） |
| WGC 不可用（HDR 屏） | 回退 DDA + `[warn]`（背景可能过曝） |
| 独占全屏应用 | 4 Hz 轮询；该屏画布隐藏，退出后恢复（R-16）+ `EngineEvent::ExclusiveFullscreen`；拖拽期间推迟处理（隐藏画布会释放 `SetCapture` 打断状态机） |
| 空屏（无块）1 s | 画布隐藏（混合刷新率保护，`hidden_empty`）；采集降到 `idle_keepalive_ms`（默认 250 ms）保活 |
| 金字塔 / RT 重建 | `invalidate()`：`pyramid_valid` 清空、下一帧整屏 Pass A |
| 句柄指向已销毁窗口 | `EngineEvent::Warning { kind: InvalidConfig }`，**不 panic、不返回 Err** |
| 上层配置非法（NaN / 尺寸 ≤0） | `create_window` 返回 `Err(ErrorKind::InvalidConfig)`（**先校验原始输入再 sanitize**） |

## 7. Good / Base / Bad

- Good：每帧自包含（整屏清 + 画全部块）；Pass A 按脏区 scissor；全透明屏整条跳过 Pass A；
  取证用连拍看时间维度。
- Base：脏区并集只 union 整块矩形（不做像素级 region），换取 O(块数) 的书本账。
- Bad：依赖 back buffer 跨帧持久（flip 模型不保）；按 dirty 的外接 bbox 清屏或裁 Pass B；
  用 loop/render 时长判丢帧；给非 hover 块不加衰减的鼠标响应；
  把缩放反馈打进 `glass_ps` 的 cbuffer（它现在是独立 pass，见 §3.4b）；在热路径读环境变量。

## 8. Wrong vs Correct

| Wrong | Correct |
|---|---|
| `ClearRenderTargetView` + 期望 scissor 生效 | 整屏清 + 画全部块（或带 scissor 的零填充绘制） |
| `quality.tick(loop_us, …)` | `quality.tick(prep+A+B+C, …)` |
| `pyramid_valid = n`（只刷了脏区∩n 却声称 n） | 只声称重算过的区域；`!covered` 先于 `backdrop_changed` |
| 非 hover 块 `normalizedMouse` 不衰减 | `nMouse = normalizedMouse * max(fall*gate, hover)` |
| HDR 屏用 DDA 的 8bit/FP16 复制面 | HDR 输出自动切 WGC |
| 影响盒只按 `3*mouse_radius` 算 | `frame_influence_px(R, grip) + ceil(margin)` |
| `ResizeEdges::TOP_LEFT.to_shader_edges()` 返回 `left\|top` | 对角 ⇒ `[0;4]`（直边掩码），只走 `to_shader_corners()` |
| 新增光标驱动效果只改着色器 | 先在 `core/geometry.rs` 登记支撑半径并纳入 `frame_influence_px` |
| 上层直接改 `Manager` / GPU 资源 | 推 `Command`，渲染线程帧首消费 |

## 9. Tests Required

`cargo test --release --workspace`（库 136 个单测 + 集成 11 + playground 5 个 schema 单测 + doctest 2 = 154）、
`python tools/gate_perf.py`（吞吐/内存硬门槛）、
`verify_dirty`（增量==全量，连跑 ≥3 次）、`repro_tremble far|many`（时间维度稳定）、
`verify_mouse_isolation`（跨块隔离）、`verify_validity`（虚报/漏画计数为 0）、
`verify_adaptive`（闭环）、`verify_crossscreen`（多屏 DPI）、`verify_env_ema`、
`cmp_visual`（跨版本稳态帧四段）。期望值与 oracle 选择见 `measurement-and-gates.md`。