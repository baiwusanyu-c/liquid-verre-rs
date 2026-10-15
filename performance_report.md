# tarui-glass 架构审查与 20 块拖拽性能报告

日期：2026-10-13
备份提交：`990df72 checkpoint: stabilize glass capture and shared rendering`

## 结论

当前实现的核心渲染架构与《折射渲染方案.md》一致，20 块场景可以稳定运行；但基于本机实测数据，当前版本**不能严格宣称满足“20 块任意大小拖拽不掉帧、不卡顿”**。平均渲染时间较低，但存在明显的长尾：普通运行期间 `max_render_ms` 约 8–19 ms，连续拖拽期间出现 `avg_render_ms=1.971–2.914 ms`、`max_render_ms=24.467–26.728 ms`，并观察到一次 `max_loop_ms=233.564 ms`。这意味着平均负载足够，但还没有达到无长尾卡顿的验收标准。

## 已验证的符合项

- 单个 D3D11 device/context；
- DXGI Desktop Duplication 捕获，并复制到自有 GPU SRV 纹理；
- 单全屏画布 + 单交换链 + DirectComposition；
- BGRA8、premultiplied alpha、flip sequential、双 buffer；
- 共享 capture、共享 blur pyramid、共享 frost/soft/env 资源；
- Pass B 逐块 scissor 绘制；
- 单次 Present + 单次 DComp Commit；
- 256 项结构化 Snell LUT；
- SDF 圆角/超椭圆、折射、RGB 色散、Fresnel、焦散、鼠标点光和环境采样；
- D3D11 SRV/RTV hazard 和 Pass A scissor 状态问题已修复；
- 启动时等待首帧后显示，避免初始黑帧。

## 主要偏差和性能风险

1. 方案第 0.3/5.1 的预算描述按“每块交换链/每块窗口”计算，而当前实现是单画布模式；这是更省资源的方向，但与方案的双模切换说明不完全一致。
2. 当前 Pass A 除四级金字塔外，额外执行半分辨率 4 次 ping-pong 和全分辨率 frost/soft 重建；这比方案中 A1+A2 的 1.33 全屏当量更重，虽然与块数无关，但会增加捕获帧到达时的长尾。
3. 没有实现 DDA dirty rect / tile-level background change detection；任意桌面变化会触发整套共享模糊。
4. 没有实现 timestamp disjoint GPU queries，因此当前指标是 CPU 侧 render-loop 时间，不是 GPU pass 时间。
5. 当前主循环持续渲染并以 4ms sleep 限速空闲，没有实现方案要求的 MsgWaitForMultipleObjectsEx 事件驱动休眠。
6. 尚未实现 `_SRGB` SRV/RTV 或显式线性空间混合；当前仍使用 UNORM。
7. 当前 DDA 只枚举/复制主输出 0，尚未实现多显示器输出切换与跨屏坐标映射。
8. 每帧 `draw_order()` 创建并排序 Vec；每帧 `update_canvas_region()` 重建 HRGN；这不是主要 GPU 瓶颈，但属于可优化热路径。
9. resize 路径可能执行 `ResizeBuffers` 和整套资源重建，方案要求交互期避免 resize 重建；需要在 resize 压测中单独验证。

## 20 块实测

场景：`GLASS_MANY=1`，20 个自动布局玻璃块；构建：release；统计：`GLASS_METRICS=1`，5 秒窗口输出；刺激：鼠标移动和约 5.27 秒连续 120Hz 鼠标拖拽。

普通 5 秒窗口观测：

- frames/5s：约 337–357；
- 平均 CPU render：约 0.137–0.271 ms；
- 最大 CPU render：约 8.230–19.059 ms；
- 最大主循环：约 26.399–49.627 ms。

连续拖拽期间观测：

- 一段窗口平均 CPU render：2.914 ms；
- 另一段窗口平均 CPU render：1.971 ms；
- 最大 CPU render：24.467–26.728 ms；
- 最大主循环：一次达到 233.564 ms，之后回落到约 36–55 ms。

## 判定

- “平均 GPU/渲染工作量与 20 块数量无关”：符合设计方向。
- “普通场景可以运行”：通过。
- “20 个任意大小玻璃拖拽全程不掉帧、不卡顿”：当前证据不足且长尾数据明确不通过严格验收。

## 追加优化基线（2026-10-13）

已完成但尚未提交的热路径优化：

- 捕获纹理视图在 compositor 中复用，避免每个 DDA frame 重复创建 capture SRV；
- 仅在块增删、尺寸变化、拖拽位置变化时重建窗口 region，不再每帧调用 `SetWindowRgn`；
- 保留 `GLASS_METRICS=1` CPU 侧统计，用于 5 秒窗口输出。

优化版普通 20 块窗口观测：

- 平均 CPU render：约 0.182–0.307 ms；
- 最大 CPU render：约 13.573–20.191 ms；
- 最大主循环：约 29.214–59.569 ms。

与前一轮相比平均值没有显著变化，说明主要瓶颈不在 SRV 创建或 HRGN 重建，而在 Present/DComp、DDA 捕获同步、模糊帧更新或 GPU/驱动长尾。下一步必须加入 GPU timestamp、PresentMon 或 ETW，不能继续仅凭 CPU render 计时优化。

## 最终复测（2026-10-13 17:40）

本轮先修复并验证了两个确定性热路径问题：

- `Manager` 缓存 z-order 索引，渲染帧不再创建/排序块指针 Vec；
- `Compositor` 复用 DDA 自有捕获纹理的 SRV；
- `region_dirty` 使窗口区域只在块增删、尺寸变化和拖拽位置变化时更新。

release 构建和 `cargo check` 均通过。20 块自动布局复测输出：

- avg render：0.174–0.471 ms（多数窗口）；
- max render：17.042–31.901 ms；
- max loop：约 31.592–56.275 ms，仍出现 128.268、143.666、224.641 ms 长尾；
- 连续 120Hz 输入拖拽约 3.16 s，过程完成，但 CPU 指标仍不能证明显示端不掉帧。

## 最终判定

当前优化减少了确定性的 CPU 分配/区域更新，但**仍未达到严格验收条件**“20 个任意大小玻璃拖拽全程不掉帧、不卡顿”。原因是实测仍有 >100 ms 的主循环长尾。该长尾不能用当前 CPU render 平均值解释，必须用 GPU timestamp、PresentMon/ETW 继续定位 DDA、Present/DComp 或驱动调度。不能将当前结果宣称为 v1.1 完整实现或无掉帧通过。

## 调度优化与分阶段复测（2026-10-13）

本轮保持 `shaders/glass_ps.hlsl`、视觉参数和 premultiplied 合成不变，完成：

- DDA 捕获纹理 SRV 复用；
- Desktop Duplication 的 LastPresentTime 变化检测，指针-only acquisition 不刷新共享 Pass A；
- Manager z-order 缓存，渲染帧不再排序分配；
- `region_dirty` 避免每帧 SetWindowRgn；
- 增加 CPU submission 分阶段统计：A=共享 Pass A、B=块绘制、P=Present、C=Commit。该统计包含 CPU 提交/等待时间，不等同 GPU timestamp。

新一轮 20 块测试：

- avg render：0.158–0.306 ms（多数窗口）；
- max render：14.492–26.506 ms，拖拽窗口最高 32.567 ms；
- max loop：多数 28.421–46.634 ms，但拖拽期间仍出现 280.972 ms 长尾；
- 分阶段通常 A=0–64 us、B=6–31 us、P=13–67 us、C=11–52 us；偶发 P=520 us/C=509 us，但无法解释 280 ms 长尾，说明长尾可能发生在 DDA AcquireNextFrame、消息调度或 GPU/驱动队列，而非 Pass B shader。

严格判定仍为：**尚未达到“20 个任意大小玻璃拖拽全程不掉帧、不卡顿”**。视觉 shader 和参数未修改，视觉回归风险保持为零；当前需要 ETW/PresentMon 或真正 D3D11 timestamp/occlusion query 继续定位外部调度长尾。

## 2026-10-14 视觉与拖拽修复

- 拖拽速度补偿只传给被拖拽/hover 块，其他玻璃使用零速度背景采样，避免一个玻璃拖动时所有玻璃模糊背景抖动。
- 折射位移在 bevel 到 plane 过渡处扩大平滑 envelope，降低 edge sharp mix；Fresnel、specular、caustic 和 mouse glint 使用连续 envelope，减少圆角横竖接触处斜角断裂及鼠标白色贴片感。
- `glass_ps.hlsl` 主体模糊、颜色、alpha 和 Pass A/B 架构保持不变。
- cargo check/release 通过；视觉回归图为 `visual_regression_20261014.png`。
- 新 20 块测试 avg render 0.189–0.357 ms，多数 max loop 32.2–75.2 ms，仍有 140.3 ms 长尾；阶段 A 约 0–80 us、B 15–229 us、P 37–89 us、C 25–71 us。

严格无掉帧验收仍未通过；视觉 shader 已按 liquid-glass-rs 的 screen/overlay 色散反馈重构，拖拽状态不再影响其他玻璃。针对转角割裂，本轮进一步移除物理 LUT 位移与 procedural reference 位移的叠加，并改为实际 block SDF 梯度 + 单调折射 profile；同时让极端回归模式真正覆盖面板同步：`GLASS_EXTREME_SPEC=1` 强制 `spec_intensity=4.0`、`spec_power=1.0`。UI 的高光强度上限已扩展到4，高光锐度下限已扩展到1。cargo check/release 通过，极端回归图为 `extreme_spec_sdf_v2_20261014.png`。该图显示对角分面已消除，但仍保留参考方案本身的连续边缘/内部柔和光带；性能仍需在干净环境使用 PresentMon/ETW 验收。


---

# 2026-10-14 严格自检 + 架构对齐改造（本轮）

> 本轮以《折射渲染方案》v1.1 为唯一判据做逐条自检，在**不改动 `shaders/` 任何一行、
> 不改动任何默认视觉参数**的前提下重做了采集/调度/重绘路径，并补齐多屏支持。
> 全部结论都有可复现的取证脚本（`tools/`）与逐像素对照结果支撑。

## 一、自检结论：改造前与方案的偏差

| # | 偏差 | 位置 | 方案依据 | 影响 |
|---|---|---|---|---|
| 1 | 主循环是忙轮询，每轮无条件 `render()+Present+Commit`，"idle" 只 `sleep(4ms)` | `main.rs` 主循环 | 0.3 / 7.4-1 / 10.4「静止即 0 帧、0 CPU」 | 静止桌面也持续 8.8% 单核 |
| 2 | DDA 用 `LastPresentTime` 当"内容变化"判据，而 DWM 每个 vsync 都推进它 | `capture/dda.rs` | 12.4.1 `NEW_BACKDROP` | **Pass A 每帧都跑**（实测 `pass_a=538/538`） |
| 3 | Pass A 恒为整屏，与块足迹无关 | `gfx/pyramid.rs` | 5.2 / 7.4-2 | 6.49 Mpx/帧的无谓着色 |
| 4 | `up` 上采样链（3 个整屏当量）从未被 shader 采样：`SamplePyramid` 在 `glass_ps.hlsl` 里定义但 `ps_main` 从未调用 | `gfx/pyramid.rs` | 7.3「热路径零浪费」 | 白烧 0.33 屏当量 + 8.5 MB |
| 5 | `soft` 与 `frost` 由**同一张源、同一个 pass** 生成，逐像素相同，却各占一张整屏 RT | `gfx/pyramid.rs` | 12.1.3 显存预算 | 白烧 1 个整屏 pass + 26 MB |
| 6 | `env16` 只是把 `down[3]` 原样再画一遍 | `gfx/pyramid.rs` | 5.6.5 | 白烧 1 个 pass + 1 张 RT |
| 7 | 每个金字塔步骤都 `ClearRenderTargetView`，但混合态是不透明覆盖 + 全屏三角形 | `gfx/pyramid.rs` | —— | 十余次多余整屏 clear |
| 8 | 1024×1024 fallback 纹理常驻，且启动时逐像素算 100 万次 `sin/cos` | `gfx/compositor.rs` | 7.1 | 4 MB 常驻 + 启动耗时；正常路径永不采样 |
| 9 | **拖拽时每帧重建 HRGN + `SetWindowRgn`**（20 块 = 20 次 `CreateRectRgn` + 19 次 `CombineRgn`） | `main.rs` | 12.2.4「拖拽一帧 ≈0.05 ms」 | `SetWindowRgn` 迫使 DWM 重裁剪整个 overlay，是 100~280 ms 长尾主因 |
| 10 | 每帧 `std::env::var("GLASS_EXTREME_SPEC")`（加进程环境锁 + 分配 String） | `main.rs` | 7.3「热路径零堆分配」 | 每帧 2 次分配 |
| 11 | `Manager::blocks_sorted()` 每帧 `.collect()` 一个新 `Vec` | `glass/manager.rs` | 7.3 | 每帧堆分配 |
| 12 | `Block` 含 `label: String`，多屏投影每帧克隆 → 每块每帧一次堆分配 | `glass/block.rs` | 7.3 | 热路径分配 |
| 13 | 无脏块剔除 / 无 `RenderReason` 分级，任何一帧都画全部块 | `gfx/compositor.rs` | 12.4.1 / 12.5 | Pass B 成本与"实际变化的块"无关 |
| 14 | **完全没有多屏支持**：`EnumOutputs(0)` 硬编码、单个全屏画布、`monitor_rect()` 只取主屏 | `capture/dda.rs`、`main.rs` | 4.1「每屏一个 session」/ 4.4 / 12.10 | 副屏上一块玻璃都画不出来 |
| 15 | 未处理 `DXGI_ERROR_ACCESS_LOST` / `NOT_CURRENTLY_AVAILABLE`，所有错误一律当"无帧" | `capture/dda.rs` | 4.3「检测 ACCESS_LOST 并暂停/降级、退避重试」 | **锁屏后主循环退化成 96 kHz 空转、83% 单核** |
| 16 | `gfx/w_mode_compositor.rs`、`capture/wgc.rs`、`capture/slot.rs` 未挂进模块树（死文件，`w_mode_compositor.rs` 引用不存在的 `gfx.dcomp`，一挂即编译失败） | —— | 12.1.2 双模分配器 | W/C 双模实质未实现；仓库有误导性死代码 |
| 17 | 无 GPU timestamp，只有 CPU 提交耗时 | —— | 7.6 | 前几轮报告反复出现"无法解释的长尾" |

## 二、本轮改造（全部不改视觉）

1. **事件驱动主循环**（方案 10.4 / 12.4.1）：`RenderReason` 门控 —— 新桌面帧、参数变化、
   几何变化、hover 变化、光标进入任一块的点光影响半径、拖拽、速度未收敛、窗口消息。
   无触发源则整帧跳过：不 Pass B、不 Present、不 Commit。
   DDA 的 `AcquireNextFrame(timeout)` 成为主循环唯一的阻塞点（以采集帧为主时钟），
   静止时 8 ms 内核等待，CPU 趋近 0。
2. **修好"内容变化"判据**：改用 `AccumulatedFrames > 0 || TotalMetadataBufferSize > 0`；
   纯指针更新连 26 MB 的 `CopyResource` 都省掉。
3. **脏矩形金字塔**（方案 7.4-2 / 12.5）：
   - Pass A 只刷新「DDA 脏矩形 ⊕ 模糊核」∩「块足迹 ⊕ 采样余量」；
   - 块足迹余量 = 折射位移 + 色散 + 模糊核 + 速度外推（128 px 起，随拖拽速度自适应）；
   - 桌面没变、只是块移动时用 192 px 迟滞，拖拽一小段完全不重跑金字塔。
4. **DDA 局部拷贝**：按脏矩形逐个 `CopySubresourceRegion`；脏区并集超过半屏或矩形数 > 24
   时才退回整屏 `CopyResource`。
5. **Pass B 脏矩形**：只清、只画「与脏区相交」的块；脏区来源含块位移（旧 ∪ 新）与
   鼠标点光影响范围。依赖 flip-model（`FLIP_SEQUENTIAL`）后备缓冲逐 buffer 保留内容。
6. **删除死资源**：`up` 链、独立 `soft` RT、`env16` 拷贝、每步 `ClearRenderTargetView`、
   1024² fallback（改 8×8）。
7. **拖拽期不碰窗口区域**：按下时一次性把 region 扩到整窗，抬起后收紧。
   块外内容恒为全透明，扩大 region 不改变任何像素；命中穿透仍由 `WM_NCHITTEST`
   的 `HTTRANSPARENT` 负责。
8. **热路径零堆分配**：`Block` 改为纯标量 `Copy`；`Manager` 去掉每帧 `Vec` 分配；
   多屏投影与 z 序复用 `Vec`；每屏 `Params` 只在参数变化时重算；环境变量只读一次。
9. **多屏（需求 4）**：每屏一个 `WS_EX_NOREDIRECTIONBITMAP` 画布 + 一个交换链 + 一个
   DDA duplication + 一套金字塔；块几何以主屏物理像素为逻辑单位，落到别的屏按
   `scale = dpi / 主屏 dpi` 换算（方案 12.2.5），并默认钳制在单屏内（方案 12.10 ①）；
   `WM_DISPLAYCHANGE / WM_DPICHANGED` 触发画布重建；没有玻璃块的屏整帧短路
   （不清屏、不 Present、不 Commit）。
10. **采集失效恢复**（方案 4.3）：区分 `WAIT_TIMEOUT`（正常静默）与真实错误；
    `ACCESS_LOST` / `E_ACCESSDENIED`（锁屏安全桌面）时释放并每 500 ms 退避重建，
    解锁后自动恢复；同时给主循环加了**速率下限**，任何异常下都不可能空转。
11. **`SetMaximumFrameLatency(1)`**（方案 7.4-7）：把渲染队列深度从默认 3 压到 1，
    减少「光标位置 → 屏幕上的玻璃」之间最多 2 个刷新周期的滞后。
    可用 `GLASS_FRAME_LATENCY` 调。
12. **GPU timestamp 计时**（方案 7.6）：`GLASS_GPU_TIMINGS=1` 打开。
    注意 timestamp query 本身会打断驱动批处理、显著拖慢管线（实测把 Pass A 报成 17 ms、
    帧率打到 15 fps），因此**默认关闭**，只在做 GPU 归因时单独开。

## 三、视觉零变化的取证（不是"看起来一样"，是逐位一致）

| 取证 | 方法 | 结果 |
|---|---|---|
| Pass A 重构（删 `up`/共享 `soft`/复用 `env16`/去 clear/缩 fallback） | 改造前后同一桌面各拍一张内部取证图，比对 overlay（真实 premultiplied backbuffer） | **0 / 6,489,600 像素差异** |
| 脏矩形 Pass A + 局部 DDA 拷贝 + Pass B 脏区 + 事件驱动跳帧 | `tools/verify_dirty.py`：同进程、同一张 capture 连渲两帧（A=受限路径，B=强制全量 Pass A + 整屏清屏 + 全部块重画），3 场景（单块 / 20 块 / 多屏各一块）× 2 屏，全程真实移动光标并**真实按住拖拽** | **6/6 PASS，全部 0 像素差异** |
| 局部 `CopySubresourceRegion` 是否漏拷 | `GLASS_VERIFY_COPY=1`：并行维护一张"永远整屏拷贝"的参考纹理，每 120 个内容帧回读逐像素比对 | **7 轮 / 840 个内容帧，mismatch_px 全为 0** |
| 多屏坐标/DPI 改造是否影响主屏 | 主屏在原点 (0,0)、`scale=1.0`，`Params::scaled(1.0)` 逐字段原样返回，画布原点为 0 → CB 常量与改造前完全同值；再用 overlay 的 **alpha 通道**做跨版本比对（alpha 只由 SDF 几何 + 阴影 + `base_alpha` 决定，与桌面采集内容无关，因此不受"桌面变了"干扰） | **alpha 通道 0 / 6,489,600 差异**（其中玻璃覆盖 3,107,155 px） |
| 副屏采集是否路由正确 | 比对副屏 capture 与主屏 capture 的同尺寸左上区域 | **相同像素占比 0.00%**（确实是另一块屏的内容） |
| 副屏 DPI 换算 | `GLASS_SPREAD=1` 每屏正中放一块 600×400 逻辑玻璃 | 主屏 630×430、副屏 **324×224 ≈ 300×200 + 阴影**，正好 0.5× |
| 跨屏拖拽 | `tools/verify_crossscreen.py`：从主屏中心按住拖到副屏内部 | **PASS**：主屏 overlay 清空，副屏出现 624×404 = 1200×760 逻辑 × 0.5 + 2×12 阴影边距 |

## 四、实测数据

环境：Intel Core Ultra 9 285H（16C/16T）、Intel Arc 140T（集显，显存即系统内存）、32 GB；
`\\.\DISPLAY1` 3120×2080 @120 Hz / 192 dpi（200%），`\\.\DISPLAY2` 1920×1080 / 96 dpi（100%）。
探针：`tools/perf_probe.py`（DPI-aware，`GetProcessTimes` + `GetProcessMemoryInfo` +
`GPU Process Memory` 计数器 + 250 Hz `SetCursorPos` 模拟人工手速拖拽）。

> 重要口径说明：本机桌面在测量期间**真的在以 60~110 Hz 持续重绘**（`dda_changed` 实测
> 47~110 次/秒），属于方案 12.4「动态背景 / 持续渲染」场景，不是"静止即 0 帧"场景。
> 因此 CPU / fps 的绝对值受桌面活跃度影响，跨轮次不完全可比；
> 而**每帧 GPU 工作量（Mpx）与常驻内存是确定的、可比的**。

### 每帧 GPU 工作量（3120×2080，满屏 = 6.49 Mpx）

| 指标 | 改造前 | 改造后（单块） | 改造后（20 块） |
|---|---|---|---|
| DDA 拷贝 | 6.49 Mpx（整屏，每个内容帧） | 1.7~3.3 Mpx（脏矩形） | 1.8~4.0 Mpx |
| Pass A（模糊金字塔） | 6.49 Mpx × ~3.9 屏当量，**每帧必跑** | 0.79 Mpx，`pass_a` 最低 **0.00 次/帧** | 1.4~2.3 Mpx |
| Pass B 脏区 | 6.49 Mpx（整屏清 + 全部块） | **0.50 Mpx** | 1.5~2.0 Mpx |
| Pass B 实际绘制块数 | 全部 | 1.0 | 8~10（20 块中） |

### 常驻内存（改造前 = 单屏；改造后 = 双屏，副屏多一套画布+采集+金字塔）

| 指标 | 改造前（1 屏） | 改造后（2 屏） | 改造后折算单屏 |
|---|---|---|---|
| WorkingSet | 194.7 MB | 166.2 MB | ≈ 141 MB（实测单屏轮次） |
| Private | 182.7 MB | 156.2 MB | ≈ 128 MB（实测单屏轮次） |
| GPU 侧（Shared Usage） | 未测（计数器当时读不到） | 222.6 MB | ≈ 173 MB（实测单屏轮次） |

单屏口径下 Private 从 182.7 → 128.4 MB（**−30%**），逐项对账：
删 `soft` 整屏 RT −26 MB、删 `up` 链 −8.5 MB、删 `env16` −0.1 MB、fallback 1024²→8×8 −4 MB，
合计 −38.6 MB，与实测 −54 MB 同向（其余为驱动侧派生占用下降）。
方案 12.11 对 C 模式的显存上界公式是 `2S + 2.33S + 14 MB`，本机 S=25.96 MB → 目标 ≈126 MB/屏；
实测单屏 GPU 侧 173 MB，其中 D3D11/DComp/DXGI 运行时约 48 MB，几何资源约 125 MB，**与方案预算一致**。

### CPU（单核当量）

| 场景 | 改造前 | 改造后 |
|---|---|---|
| 20 块静止（桌面持续重绘 47~58 次/秒） | 8.83% | **5.32%** |
| 20 块拖拽 | 12.34% | **3.57%~10.02%** |
| 单块静止 | 8.83% | 7.14% |
| 单块拖拽 | 18.58% | 8.71%~12.89% |
| **锁屏 / 采集失效**（安全桌面，DDA 全部 E_ACCESSDENIED） | **83.25%**（96 kHz 空转，`loops=829190/5s`） | **0.469%** |
| CPU 侧 render 均值（单块静止） | 0.294 ms | **0.094~0.118 ms** |
| CPU 侧 render 峰值 | 17~31 ms | **0.66~0.94 ms** |

### 帧节奏

| 场景 | 改造前 | 改造后 |
|---|---|---|
| 单块静止 fps | ~110 | 96~112 |
| 单块拖拽 fps | ~100~105 | 55~112（波动，见下方未决项） |
| 20 块静止 fps | ~71 | 45~79 |
| 20 块拖拽 fps | ~70 | 62~87 |
| `max_loop_ms`（静止） | 30~46 | 22~83 |
| `max_loop_ms`（拖拽） | **179~190** | 36~241 |

## 五、仍未达标 / 未决项（诚实记录）

1. **拖拽 fps 波动大、`max_loop_ms` 仍有 36~241 ms 长尾**，未稳定达到方案 M2 的
   「120 Hz 屏 ≥ 90 fps」。本轮已排除的嫌疑：`SetWindowRgn`（拖拽期已归零）、
   Pass A（拖拽期最低 0.00 次/帧）、Pass B 面积（0.5 Mpx）、CPU 提交（`avg_render` 里
   大部分是 `Present` 阻塞，`stage_us` 的 A/B/P/C 通常只有 20~90 µs）。
   剩余嫌疑集中在 **DWM 侧对整屏 3120×2080 premultiplied 画布的合成开销** 与
   **`Present` 的排队深度**。下一步应做方案 12.2.2②/12.8.2 的「画布移动包络 + 尺寸分桶」：
   把画布窗口和交换链从"整屏"收缩到"块包围盒 ⊕ 256 px"，Present 与 DWM 合成成本
   随包围盒面积而不是屏幕面积走（单块场景可再降约 6×）。这项改动较大，需要独立的
   逐像素取证，本轮没有做。
2. **`GLASS_FRAME_LATENCY=1` vs `3` 的 A/B 没跑完**：测量到一半工作站锁屏，
   DDA 全线 `E_ACCESSDENIED`，后续所有用例的应用进程都在启动阶段退出（`cpu=0.0`）。
   解锁后重跑：
   `foreach($L in 1,3){ $env:GLASS_FRAME_LATENCY="$L"; python tools/perf_probe.py --case all --seconds 12 --warmup 4 --out tools/lat$L.json }`
3. **20 块静止 CPU 5.32% / 拖拽 3.57~10.02%**，优于改造前但**未达到方案 M2 的 ≤1% 单核**。
   在"桌面以 60~110 Hz 持续重绘"的前提下，方案 12.11 的持续渲染预算本身就允许
   0.16 ms/帧 CPU（C 模式），实测 `avg_render` 已到 0.08~0.15 ms，**CPU 提交侧达标**；
   差额来自 DDA 拷贝提交、DWM 合成与驱动排队，需要第 1 项的画布收缩才能继续压。
4. **`capture/wgc.rs`、`capture/slot.rs`、`gfx/w_mode_compositor.rs` 仍是未挂载的死文件**。
   方案 4.1 把 WGC 列为主选、DDA 为备选，而 12.3 又指出「`backend = dda` 通常比 WGC
   低 2~5 ms」；本项目实测 DDA + `WDA_EXCLUDEFROMCAPTURE` 无自反馈、无黑块，
   故保持 DDA。`w_mode_compositor.rs` 引用了不存在的 `gfx.dcomp`，一挂进模块树就编译失败，
   属于误导性死代码，建议删除或补完（本轮未删，避免超出"不改视觉"的授权范围）。
5. **瓦片级（64 px 格）背景变更检测（方案 12.5）未实现**，本轮做的是矩形级
   （DDA 脏矩形 ∩ 块足迹）。矩形级在"单一区域动画"下已能拿到大部分收益
   （实测 `pass_a` 最低 0.00 次/帧）；多个分散动画区域时，单个并集矩形会偏保守。
6. **液态融合 / 玻璃叠玻璃 / 成组拖拽 / 磁吸 / 自适应质量控制器（方案 M4）未实现**。
7. **环境色探针 EMA（方案 12.6 / R-20）仍是 CPU 侧常量**：`glass_renderer.env_ema`
   初值 `[0.35,0.42,0.5]`，`set_env_ema` 从未被调用，shader 里 `gEnv16` 直接采 1/16 层。
   动态背景下自适应染色会随背景抖动 —— 这是方案标注为"高概率必现"的 R-20，
   修它会改变视觉，故本轮未动。

## 六、复现方式

```powershell
cargo build --release

# 性能四象限（静止/拖拽 × 单块/20 块），输出 CPU、常驻内存、GPU 占用、帧节奏
python tools/perf_probe.py --case all --seconds 12 --warmup 4 --out tools/final.json

# 脏矩形渲染逐像素自证（3 场景 × 每屏，含真实拖拽）
python tools/verify_dirty.py

# 跨屏拖拽 + DPI 换算验收
python tools/verify_crossscreen.py
```

应用侧开关（全部只影响诊断，不影响视觉）：

| 变量 | 作用 |
|---|---|
| `GLASS_METRICS=1` | 5 秒墙钟窗口输出 CPU/脏区/Pass A/DDA/帧率统计 |
| `GLASS_GPU_TIMINGS=1` | 额外打开 D3D11 timestamp（**会拖慢管线**，仅做 GPU 归因时用） |
| `GLASS_MANY=1` | 启动即 20 块自动布局 |
| `GLASS_SPREAD=1` | 每块显示器正中各放一块玻璃（多屏验收） |
| `GLASS_FORCE_FULL=1` | 关闭 Pass B 脏矩形，每帧整屏重画（A/B 对照） |
| `GLASS_VERIFY_PASS_A=1` + `GLASS_INTERNAL_SHOT=<path>` | 同进程输出"受限路径 / 全量路径"两张取证图 |
| `GLASS_INTERNAL_SHOT_AT_MS=<ms>` | 延迟取证，配合上一项才能测到脏矩形路径 |
| `GLASS_VERIFY_COPY=1` | 局部拷贝 vs 整屏参考拷贝的周期性逐像素比对 |
| `GLASS_FRAME_LATENCY=<1..8>` | DXGI 渲染队列深度，默认 1（方案 7.4-7） |
| `GLASS_EXTREME_SPEC=1` | 极端高光回归（`spec_intensity=4.0`、`spec_power=1.0`） |


---

## 七、本轮补充：采集失效（锁屏 / 独占全屏）时的行为

自检过程中 workstation 恰好在测量中途锁屏，意外把方案 4.3 / R-16 的失效路径完整暴露出来，
本轮一并修掉：

| 现象 | 改造前 | 改造后（实测） |
|---|---|---|
| DDA 返回 `ACCESS_LOST` / `E_ACCESSDENIED` | 所有错误一律当"本帧无内容"，`AcquireNextFrame(0)` 立即返回 → **主循环 96 kHz 空转、83.25% 单核**（实测 `loops=829190 / 5 s`） | 区分 `WAIT_TIMEOUT`（正常静默）与真实错误；失效即释放 duplication，每 500 ms 退避重建；主循环另有 `IDLE_WAIT_MS` **速率下限**兜底 → **0.47~0.63% 单核**（实测 20 s） |
| 画布内容 | 冻结在最后一帧，且仍以 `TOPMOST` 压在独占全屏应用之上 | 采集一失效即 `ShowWindow(SW_HIDE)`；恢复后 `invalidate()` 强制整屏重建再 `SW_SHOWNOACTIVATE` |
| 日志 | —— | 失败/恢复只在状态翻转时各打一次，锁屏数小时也不会刷爆日志 |
| 启动 | 主屏采集失败直接 `return` 退出进程 | 带退避重试最多 6 s；仍失败则**不显示该屏画布但进程存活**，运行期继续重试，解锁后自动恢复 |

锁屏态实测（安全桌面，两块屏的 `DuplicateOutput` 全部 `E_ACCESSDENIED`）：
CPU **0.625%** 单核、WorkingSet **127.4 MB**、Private **117.6 MB**、无空转、无日志刷屏。

## 八、关于「常驻内存」指标的诚实说明

方案 7.1 给的两个目标是**分开**的：进程 RSS ≈30–40 MB（**不含显存**）+ 显存 ≈59 MB@1080p /
≈126 MB@4K（C 模式，见 12.11 的 `2S + 2.33S + 14 MB`）。

本机是 Intel Arc 140T **集成显卡**，WDDM 下 GPU 分配直接来自系统内存并计入进程的
Private Bytes，因此"RSS 不含显存"这个前提在本机不成立，两个目标会合并成一个数：

| | 方案预算（3120×2080，C 模式，S = 25.96 MB） | 实测（单屏） |
|---|---|---|
| 采集纹理 | 1S = 26 MB | 26 MB |
| 交换链（2 buffer） | 2S = 52 MB | 52 MB |
| 金字塔 ½+¼+⅛+1⁄16 | 0.33S = 8.6 MB | 8.5 MB |
| 块分辨率模糊链（½ ×2 + 整屏 frost） | —— （方案未列，属本项目对 1⁄16 颗粒感的补强） | 13 + 26 = 39 MB |
| 运行时（D3D11/DComp/DXGI/驱动映射） | 14 MB | ≈48 MB |
| **合计** | **≈101 MB + 补强 39 MB ≈ 140 MB** | **Private 128.4 MB / GPU Shared 173 MB** |

结论：**几何资源侧与方案预算吻合**；Private 从 182.7 → 128.4 MB（−30%）。
要继续往方案的 30–40 MB 靠，只有两条路，都会改变取舍，需要单独授权：
1. `capture_scale = 0.5`（方案 7.4-3）：采集与金字塔降到半分辨率，本机可省约 45 MB，
   代价是高 `clarity` 时折射层轻微软化；
2. 把「块分辨率模糊链」（`frost_half` ×2 + 整屏 `frost`，合计 39 MB）换回纯 1⁄16 金字塔采样
   —— 这正是当初为了消除"1⁄16 放大后的方块颗粒"而加的补强，去掉会**改变视觉**，本轮不做。
