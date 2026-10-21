# 图形/管线改动检查清单（改前过一遍）

> 重构后是 Cargo workspace：库在 `crates/liquid-verre/`，被测 EXE 是
> `target/release/liquid-verre-playground.exe`（`tools/*.py` 可用 `VERRE_EXE` / `VERRE_OLD_EXE` 覆盖），
> **库默认不自建窗口**，取证脚本一律显式设 `GLASS_DEFAULT_BLOCK=1`。

1. 读 `backend/rendering-pipeline.md` 的 9 条不变量与 `graphics-d3d11-gotchas.md`。
2. 问：这个改动影响**时间维度**（逐帧稳定）还是**空间维度**（单帧像素）？
   - 时间 → 需要连拍 oracle（`repro_tremble far|many` / `GLASS_SHOT_EVERY_MS`）。
   - 空间 → 需要跨版本逐像素 oracle（`cmp_visual.py` 的 **deep / interior / ring / outside 四段**，
     比**稳态帧** `GLASS_INTERNAL_SHOT_AT_MS=2500`，光标物理停放在离玻璃 > 3*mouse_radius 处）。
3. 问：脏区/有效性账本是否仍诚实？（只声称重算过的；三来源并集不短路；
   全透明屏既不重算也不声称）
4. 问：HDR 屏与 SDR 屏、DDA 与 WGC 两条路径都覆盖了吗？（两条路径的格式/色彩空间语义不同）
5. 问：debug 构建会 panic 吗？（外部整数溢出 / `Instant` 下溢 / NaN 进 GPU）
   跑一次 `cargo run --release --bin liquid-verre-playground`，再跑一次不带 `--release` 的。
6. 改 kernel/scissor：重算幂等外扩常量（`DOWN_GROW=[14,12,8,0]`、tent `32/24/16/8`、
   `BLUR_REACH=96`、`HYSTERESIS=192`、`pad = 128 + min(vel*max_lag,128)`）并跑 `verify_dirty`
   **≥3 次**（间歇残差只会在多次运行里露头）。
7. 性能结论：轮转 A/B + ≥3 重复 + 中位数；先验机器健康（`rel_max<5ms`、`acq_max<25ms`）。
   **取证期间不要同时构建**（另一个 agent 跑 `cargo build`/`cargo test` 就会把窗口打成劣化）。
8. 提交信息写清：改了什么不变量、用什么 oracle 验的、数字是多少。

## 追加三问（重构后新增的高危面）

9. **改 CB 槽位了吗？**
   - `c0..c17` 的语义是**冻结**的 —— 它们是历史取证图的逐位比对基准，改一个字段就等于作废
     全部跨版本对比。新增语义一律往后加（现状：`c18` = mode/opacity/handleGrabbed）。
     缩放反馈 border 的掩码/颜色**不**在这个 cbuffer 里，见下一条。
   - 预留槽**必须写零**：dynamic cbuffer 的 `DISCARD` 映射会读到脏数据。
   - 三处必须同步：`shaders/glass_ps.hlsl` 的 cbuffer 注释、`gfx/glass_renderer.rs` 的
     `CB_FLOATS`(= 19*4) 与 `cb.set4(...)`、`backend/rendering-pipeline.md §3.4` 的语义表。
  - 改 border：`shaders/border_vs.hlsl` / `border_ps.hlsl` 的 `BorderCb`、
     `gfx/glass_renderer.rs` 的 `draw_borders`、`rendering-pipeline.md §3.4b` 三处同步。
   - 新增**光标驱动**的效果时：先在 `core/geometry.rs` 给出它的有限支撑半径并纳入
     `frame_influence_px`，**再**写着色器；顺序反了就是下一次拼接缝（不变量 5 的前科）。
     掩码若分直边/对角两类，必须在**源头互斥**（`to_shader_edges` 对角全 0、
     `to_shader_corners` 单边全 0），否则切点附近会叠出亮斑。
10. **改默认参数了吗？**
    - `LiquidParams::default()` 就是**视觉基线**。特别是 `tint_opacity: 0.01` 与
      `blur_min: 20.0 * 0.12` —— 这两个「看起来像手误」的数字是旧二进制**稳态帧**真正渲染的值
      （旧面板 `Arc<AtomicIsize>` 量化截断 + 派生所致；旧二进制引导帧与稳态帧实测相差
      873748 像素 / 最大通道差 140）。
    - ⚠ `blur_min` **必须写成表达式** `20.0 * 0.12` 而不是字面量 `2.4`：f32 下前者
      `= 2.3999999`(`0x40199999`)、后者 `= 2.4000001`(`0x4019999A`)，**差 1 ULP**，
      用字面量会多出约 2400 个 ±1 LSB 差异像素（interior 687 + ring 1679 + outside 4）。
      rustfmt / clippy 不会动它，但「顺手化简」会 —— 别化简。
    - 改任何默认值都要：更新 `BASELINE.md` 指标表 + 重跑 `cmp_visual.py`（single 四段必须全 0）
      + 在 `CHANGELOG.md` 记一条 Changed/Fixed + 说明是不是有意的行为变更。
    - 新增字段还要同步 `playground/src/schema.rs::liquid_fields()`（否则单测 FAIL），
      并确认 `sanitize()` 的夹紧区间与 schema 的 min/max/step 一致。
11. **改 hover 相关视觉了吗？**（鼠标点光 / 缩放 border / 邻近余晖 / 拖拽速度补偿）
    - **隔离性**是硬判据：只有被 hover 的那一块可以有非零反馈，其余块必须**逐位不变**
      （`verify_mouse_isolation` / `repro_tremble far|many`）。检查点：`DrawBlock::feedback`
      是否对非 hover 块为 `ResizeFeedback::NONE`、`c12` 的 hover/press 是否只对 hover 块非 0、
      `c13` 的拖拽速度是否**仅被拖块**非零（打到全部块会让其它玻璃背景一起晃）。
    - **影响盒**是否仍严格覆盖新效果的支撑窗：出口只有一个
      `MonitorRt::influence_px() = frame_influence_px(mouse_radius, max(grip,1)) + ceil(shadow_margin)`，
      其中 `frame_influence_px = max(3R+32, 2*grip+16)`。消费者：`gfx/compositor.rs` 脏区推导、
      `verify_validity` 自检、`runtime/mod.rs` 的 `mouse_relevant`。**不许**在任何一处手写外扩。
    - **取证口径**：光标必须停在离任何玻璃 > `3*mouse_radius`（默认 660 px）的地方，
      否则 hover 效果本身会让跨版本对比对不上（实测：停在块内时整片圆角最大通道差 140）。
      `GLASS_CURSOR_AT` 现在**全程钉死**（不再只设初值），窗口过程与主循环都必须走
      `App::read_cursor()`。
    - 命中带伸出块外的效果（缩放把手）要同时检查 `MonitorRt::region_margin()`
      （= `max(margin, max_grip + 2)`）是否跟着外扩，否则那部分鼠标消息根本到不了我们窗口。
    - **只改 border 的宽度 / 不透明度 / 颜色**（`ResizeFeedbackOptions`；0.4.1 起默认
      `border_width 5.0` 逻辑像素 + `alpha 1.0` + 纯白，此前是 `3.0 / 0.95`）时
      **不需要**重算影响盒：band 用的是 SDF **内侧**距离（`inside = max(-sd, 0)`、
      `aaBand = min(aa, bw * 0.5)`、`band = 1 - smoothstep(bw - aaBand, bw, inside)` 实芯 + 外沿 AA），画在玻璃形状内部，支撑窗仍只由 `grip` 决定；
      也不动 `c0..c18`（宽度 / 不透明度 / 颜色走独立的 `BorderCb.b4/b5`）。
      注意 `cmp_visual.py` **看不到**这类改动（取证时光标停放在离玻璃 > `3*mouse_radius` 处，
      hover 与把手都不命中 ⇒ `feedback == ResizeFeedback::NONE`），只能用连拍
      `GLASS_SHOT_EVERY_MS` + 肉眼，或把光标物理停放到把手上验。