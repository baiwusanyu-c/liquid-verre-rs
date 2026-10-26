# Research: 架构路线再审——「画布+块仿真窗口」vs「每块一个真 Win32 窗口承载玻璃」

- Query: 评估「每块一个真 HWND」替代当前「每屏一个全屏画布」方案的可行性、性能、工作量与风险
- Scope: 内部代码审计 + 外部 Windows 平台事实查证
- Date: 2026-10-26

---

## 执行摘要（≤20 行，直接回答三个子问）

### 1. P0-2+P0-1+P1 做完算不算「重造窗口」？

**不算。** 当前 P0 清单只补三件事：位图挂载点（`ContentStore`，每块一张 `ID3D11Texture2D`，宿主推像素）、事件转发（`GlassInputSink` trait + 按压仲裁状态机 ~250 行）、焦点路由（P1，~30 行）。这三者加起来 ≈ 一个**极简位图按钮**，离「窗口」还差：键盘路由（IME/TAB/快捷键）、UIA 无障碍、拖放注册（`RegisterDragDrop`/`IDropTarget`）、DWM 阴影/圆角动画、Alt-Tab 缩略图、任务栏条目、Z-order 管理。这些是 HWND 天然提供的，但也是宿主壳窗口的责任——库不需要也不应该承担。

### 2. 切换为 per-block HWND 是否「方案·视觉·性能」三致？

**方案不一致**：架构从「单 device + 单 swapchain + 单 Present + 单 Commit」变为「单 device + N 个 swapchain + N 次 Present + N 次 Commit」。**视觉一致**（同构条件成立——见 §3 分析）：当前玻璃只折射桌面金字塔（永不折射其他玻璃块），per-window 方案排除自身即语义一致；但影响盒外扩导致窗口必须大于玻璃矩形，透明死区 + 逐块 NCHITTEST 穿透的复杂度远超现状。**性能不一致**：N 个 swapchain 的 DWM 重定向表面显存按 N 线性增长，DWM 合成 N 层 overhead 不再是单层；BASELINE 的 9 条不变量中至少 4 条（#1 每帧自包含、#3 模糊链幂等、#6 脏区三来源并集、#8 自适应判据）需要重定义。

### 3. 难度是否更小？

**否。** 剩余路线图工作量 ≈ P1 30 行 + P2 50 行 + 验证；窗口化重写涉及渲染器重构（~2000+ 行）、输入模型重设计（~500+ 行）、DComp 视觉树重构、BASELINE 全量重取证（8 个脚本 × 多轮）、迁移双轨维护（0.x→2.0 不兼容升级）。结构上，现状是「在已知且已验证的系统上加 80 行业务逻辑」；窗口化是「拆掉已验证的渲染管线，在 20 个窗口上重新证明它的正确性」。

**推荐路线：混合方案 C——画布继续做视觉，承认库边界「库只做玻璃，窗口本体永远归宿主」。** 当前 P0 清单已覆盖宿主真正缺的三件事；剩余天花板（键盘/IME/UIA/拖放/DWM 动画）是宿主壳窗口的责任，不是库的。

---

## 对比表

| 维度 | 现状：单画布+块仿真 | 变体 A：每块一个库 HWND | 变体 B：每块一个宿主 HWND | 混合方案 C（推荐） |
|---|---|---|---|---|
| 输入路由 | P0-2 状态机 ~250 行；未覆盖键盘/IME/DnD | OS 原生；但焦点抖动/UAC 前台锁/DWM 过渡干扰 | 同 A，且需跨进程 sink 协议 | 同现状 + P1 焦点路由 ~30 行；键盘/IME/DnD 归宿主壳窗口 |
| backdrop 采集 | per-monitor 一次 DDA，共享金字塔 | N 窗口需共享一份 capture；OpenSharedResource 可行但需 keyed mutex 同步 | 宿主不负责采集，同 A | 不变（已验证） |
| 玻璃块互相折射？ | **不折射**（只采桌面金字塔）→ per-window 语义一致 | 一致（排除自身窗口即可） | 一致 | 不变 |
| 影响盒外扩 | 画布局部整数运算，一次 SetWindowRgn | N 个 HWND 各自扩到影响盒尺寸，透明死区大 | 同 A | 不变 |
| 点击穿透 | 一次 NCHITTEST→HTTRANSPARENT + 逐帧 WS_EX_TRANSPARENT | N 次逐窗口 NCHITTEST，死区穿透逻辑复杂 | 同 A | 不变 |
| swapchain/Present/Commit | 1 次/屏 | N 次/屏 | N 次/屏 | 1 次/屏 |
| 遮挡剔除 | B1 8 Hz EnumWindows，单画布 | 需 N 次 EnumWindows，或共享扫描结果 | 宿主窗口自行管理 | 不变 |
| 显存（20 块） | ~160 MB Private | 估计显著增加（N 份 DWM 重定向表面） | 同 A | ~160 MB |
| 性能基线 | many-drag 70-74 fps (默认) / 105 fps (lat=2) | 需全量重测；预计显著退化 | 同 A | 不变 |
| 不变量兼容 | 9+3 条全部成立 | #1/#3/#6/#8 需重定义 | 同 A | 全部成立 |
| 迁移工作量 | 0（现状） | ~2000+ 行渲染 + ~500+ 行输入 + 全量重取证 | 同 A + 跨进程协议 | P1 30 行 + P2 50 行 |

---

## 逐项分析

### 1. 输入/焦点/键盘/IME/Alt-Tab/任务栏/拖放

**现状**（`platform/window.rs:101`）：
- 画布 `WS_EX_NOACTIVATE` —— **故意不抢焦点**，否则打断宿主输入链
- 输入仅处理 5 条消息：`WM_NCHITTEST`（返回 `HTTRANSPARENT` 穿透）、`WM_LBUTTONDOWN/UP`、`WM_MOUSEMOVE`、`WM_SETCURSOR`
- `WS_EX_TRANSPARENT` **逐帧切换**（`window.rs:432-443`）：块内 Down 时临时关掉穿透以接收 Move/Up
- 键盘消息全部走 `DefWindowProc`，ESC 路径自认「正常拿不到焦点」（`window.rs:236-244`）
- 拖放无 `RegisterDragDrop`/`IDropTarget`；IME/UIA 完全不存在；Alt-Tab 缩略图归宿主壳窗口

**per-HWND 方案的「省下」**：
- `WM_NCHITTEST`、`SetCapture`、`WM_MOUSEMOVE` 转发出 OS 接管 → 省掉 P0-2 的 ~250 行状态机
- 拖放/DnD → OS 原生 `RegisterDragDrop`

**per-HWND 方案的「新增坑」**：

1. **焦点抖动**：当前 `WS_EX_NOACTIVATE` 保证画布永远不会抢焦点。真 HWND 可激活意味着：
   - 用户在宿主窗口打字 → 鼠标划过玻璃块 → `SetFocus` 切换 → IME 上下文销毁重建 → 输入法候选窗闪烁
   - UAC 前台锁：`AllowSetForegroundWindow` 限制下激活可能被系统静默拒绝
   - 最小化动画/DWM 过渡：`WS_POPUP` 变 `WS_OVERLAPPED` 产生窗口动画

2. **Alt-Tab 污染**：每块一个 HWND → 20 块 = Alt-Tab 列表多 20 个条目（除非加 `WS_EX_TOOLWINDOW`，但这会丢失任务栏项；且 `WS_EX_TOOLWINDOW` 窗口仍然出现在 Alt-Tab 的「桌面」分组）

3. **拖拽竞态**：逐帧 `WS_EX_TRANSPARENT` 切换在 per-block 场景变成 N 个窗口各自维护穿透状态，OLE `DoDragDrop` 的 `WindowFromPoint` 竞态窗口从 1 变 N。

4. **SetCapture 冲突**：当前一块 Down 时 `SetCapture(hwnd_canvas)`，整个画布接收 Move/Up。per-block 方案需 N 个独立 `SetCapture`，但系统同时只有一个捕获窗口——一块拖动时另一块的 hover 检测会断。

**来源**：`platform/window.rs` 行 1-450（窗口创建/消息路由/穿透机制）、`input/cursor.rs`（`GetCursorPos` 轮询）、`core/routing.rs`（按压仲裁状态机）。

**省下工作量**：P0-2 ~250 行状态机 + P1 焦点路由 ~30 行 = ~280 行。
**新增工作量**：焦点管理（`SetFocus`/`WM_ACTIVATE`/`WM_KILLFOCUS` 协调 N 个窗口）、IME 上下文管理、Alt-Tab 过滤、拖放竞态处理，估计 500+ 行 + 深度集成测试。

---

### 2. Backdrop 采集

**关键事实（代码证实）**：

当前 `Pass B` 的玻璃像素着色器采样的是 **桌面金字塔**（`Pyramid` 从 `capture_srv` 构建），**不是** RT 里已画好的下层玻璃块。证据链：

1. `compositor.rs:429-480`：`render()` 接收 `capture_srv`（DDA/WGC 桌面采集），Pass A 从 capture 构建金字塔
2. `glass_renderer.rs:217-240`：`draw_blocks()` 接收 `pyramid: &Pyramid` 和 `capture: &ID3D11ShaderResourceView`——都来自桌面，不是 backbuffer RT
3. `pyramid.rs` 全模块：下采样链 capture→1/2→1/4→1/8→1/16，frost/soft/env 全部从桌面派生
4. Block z-order 只影响 draw order（scissor 顺序），不影响着色器采样源

**结论：当前架构中玻璃块永不折射其他玻璃块，只折射桌面背景。** 这与 per-window 方案（每窗口只采桌面、排除自身）语义完全一致——「视觉一致」的同构条件成立。

**per-monitor DDA 共享给 N 个窗口的可行性**：

- `ID3D11Device::OpenSharedResource`（MSDN）需要源纹理创建时带 `D3D11_RESOURCE_MISC_SHARED` 标志。DDA `AcquireNextFrame` 返回的纹理由 DXGI 运行时管理，**不保证**支持共享句柄导出。
- 可行路径：采集线程维护一份 capture texture → 通过 `IDXGIResource::GetSharedHandle` 导出共享句柄 → N 个渲染窗口各自 `OpenSharedResource` + `IDXGIKeyedMutex` 做帧级同步。
- **已知实践**：CEF offscreen rendering + D3D11 共享纹理已验证跨进程/跨设备纹理共享链路，但 DDA 源纹理的特殊性（DXGI 内部分配、可能使用 staging surface）使得此路径**未经本项目实测验证**，属于不确定项。
- 若不共享，则每窗口需独立的 DDA session——但 DXGI Desktop Duplication 是 **per-output 全局唯一**（一个 HMONITOR 同时只允许一个 duplication），第 2 个窗口尝试 `DuplicateOutput1` 会直接失败。因此 N 窗口**必须共享**同一份采集结果。

**`WDA_EXCLUDEFROMCAPTURE` 对 DComp 窗口**：
- `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)`（`window.rs:124-130`）在 DDA/WGC 两端都生效。DComp 内容窗口的行为与当前画布无区别——DComp visual 附着在 HWND 上，`WDA_EXCLUDEFROMCAPTURE` 作用于 HWND，因此**同样有效**。

**来源**：`compositor.rs:429-660`、`glass_renderer.rs:132-350`、`pyramid.rs:1-100`、`capture/mod.rs:1-80`、MSDN `ID3D11Device::OpenSharedResource`、DXGI Desktop Duplication API 文档。

---

### 3. 鼠标光效/悬停 border/阴影/模糊触达超出块矩形

**现状**（`BASELINE.md` 不变量 5、11，`compositor.rs:260-280`）：
- 影响盒半径 `MonitorRt::influence_px() = max(3R+32, 2*grip+16) + ceil(shadow_margin)`
- 3R = 3 × mouse_radius（默认 ~48 → 144 px）；grip = 缩放反馈把手半径（默认 16 → 48 px）
- 当前画布把影响盒外扩到 `SetWindowRgn` + `NCHITTEST` 穿透，透明死区归画布的一条 HRGN

**per-HWND 方案**：
- 每个 HWND 必须扩到 `block_rect ⊕ influence_px`，否则光标靠近边缘时 border/光效被窗口边界裁掉
- 20 块 × 每块外扩 ~200 px（3R+32+margin）→ 大量透明死区
- 每块需独立 `NCHITTEST→HTTRANSPARENT` 对死区穿透 + 对块内非交互区穿透
- **拖拽时影响盒跟随**：`SetWindowRgn` 每帧重建 N 次（当前有拖拽期零 `SetWindowRgn` 优化，`window.rs` 注释行 6-8：旧实现每帧 `SetWindowRgn` 导致 100~280 ms DWM 重裁剪长尾）
- **hover 只亮一块**：`to_shader_edges` / `to_shader_corners` 在着色器里做掩码，per-window 无影响

**连锁破坏**：
- BASELINE 不变量 5（鼠标影响 3σ 有限支撑）需要改为「每窗口独立计算影响盒，跨窗口边界不传播」
- `verify_mouse_isolation` 取证脚本需重写：当前验证「拖一块其余块 0 像素变化」依赖同一画布同一 RT，per-window 后变成 N 个独立 RT，像素比对需跨 swapchain 截图后拼合

**来源**：`core/geometry.rs`（`frame_influence_px` 单一来源）、`platform/window.rs:6-8`（拖拽期零 SetWindowRgn 优化注释）、`compositor.rs:260-280`。

---

### 4. 性能红线（用户第一门禁）

**当前性能特征**（`BASELINE.md`，3120×2080@120Hz + 1920×1080@60Hz，Intel Ultra 9 285H / Arc 140T）：

| 场景 | fps / p50 | 我们自己的工作 | 显存 Private |
|---|---|---|---|
| 单块拖拽 | 113-116 / 8.32-8.36 ms | 30-80 µs | ~160 MB 双屏 |
| 20 块拖拽（默认 lat=1） | 70-74 / 13.0-13.2 ms | 40-84 µs | ~160 MB |
| 20 块拖拽（lat=2） | 105.0 / 8.34 ms | 同 | ~160 MB |
| 静止 idle | 0 帧 0 CPU | 0 | ~160 MB |

关键成本归因（`rendering-pipeline.md` §2.1）：DDA 采集 + DWM Present 阻塞合计 10-30 ms，我们自己的工作（prep+PassA+PassB+Commit）仅 30-84 µs——**我们自己不是瓶颈**。

**per-HWND N 个 swapchain 的量化判断**：

1. **后备缓冲显存**：flip-model swapchain 每窗口至少 2 个 buffer（`BufferCount=2`）、假设平均 600×500 像素（含影响盒外扩）`B8G8R8A8_UNORM`。20 块 × 每块 600×500×4 bytes × 2 buffer = **48 MB** 后备缓冲。加上共享金字塔（当前 ~52 MB），显存 ≈ **100 MB**——低于当前 160 MB。但 DWM 为每窗口额外维护重定向表面（redirection surface），成本 ≈ 窗口面积 × 4 bytes × 2，约**额外 48 MB**。总计 **~148 MB**，仍在 160 MB 红线内。

2. **Present/DWM 合成开销**：flip model 下 DWM 直接从 swapchain buffer 合成（MSDN），不产生额外 blit。但 DWM 合成 N 层（vs 当前 1 层）在处理脏矩形和遮挡计算时的 CPU 成本线性增长。DWM 合成线程是共享资源，N 层窗口对系统整体合成延迟的影响不可忽略。

3. **N 次 DComp Commit**：`DComp::commit()` 当前每屏 1 次。per-window 变 N 次，每次 Commit 触发 DWM 重合成。N=20 时 Commit 自身（~5-10 µs/次）不是瓶颈，但 20 次独立的 DWM 合成触发会使 DWM 的批处理优化失效。

4. **遮挡剔除**：B1 当前 8 Hz 每屏扫描一次 `EnumWindows`。per-window 方案要么 N 个窗口各扫一次（N× 成本），要么共享扫描结果（跨窗口通信成本）。

5. **不变量影响**：
   - #1（每帧自包含）：当前 flip-model 下 back buffer 在 Present 后内容不保，每帧整屏清+画全部块。per-window 同理但范围缩小到窗口矩形。
   - #3（模糊链幂等）：金字塔从全屏 capture 构建，仍共享。但 Pass A 的 scissor 范围需按 N 个窗口的足迹并集计算——要么在采集线程集中做（跨线程通信），要么每窗口独立做（N 次冗余）。
   - #6（脏区三来源并集）：桌面脏区 + 位移 + 影响盒——每窗口独立计算，但「位移」需跨窗口通信（一块移动可能影响相邻窗口的阴影重叠区）。
   - #8（自适应判据）：`own_us` = prep+PassA_N+Σ(PassB_i+Commit_i)/N——不再是从一个画布上测量。

**来源**：MSDN DXGI flip model、NVIDIA swap chain best practices、`compositor.rs` `FrameInput` 结构体、`pyramid.rs` 金字塔构建、`BASELINE.md` 阀门指标。

---

### 5. 工作量与风险

**剩余路线图（当前路径）**：

| 项 | 预估 | 状态 |
|---|---|---|
| P1-1 焦点路由 | ~30 行 `core/routing.rs` 扩展 + `engine.rs` 收口 | 待做 |
| P1-2 拖入验证 | 文档+验证脚本（代码仅 `api/event.rs` 补事件变体） | 待做 |
| P2 move filter | ~50 行 `api/config.rs` + `runtime/commands.rs` | 待做 |
| 取证脚本 | content 1 个、input 1 个 | 待做 |
| playground demo | 展示内容层+输入路由的示例 | 待做 |

**窗口化重写（per-HWND 方案）**：

| 模块 | 预估 | 说明 |
|---|---|---|
| 渲染器 ×N | 1500-2000+ 行 | `Compositor` 从单画布改为 N 窗口管理器；每窗口独立 swapchain/DComp visual；共享金字塔 + keyed mutex 同步 |
| 输入模型重设计 | 500+ 行 | `platform/window.rs` 消息路由从「一个 wnd_proc 管全屏」变 N 个独立 wnd_proc + `SetCapture` 冲突解决 |
| DComp 视觉树 | 200+ 行 | 可能需要在宿主进程中创建 DComp device 并跨设备合成 visual（MSDN：不同 device 的 visual 可在同一树中合成，各自独立 Commit） |
| 共享采集 | 300+ 行 | DDA 采集线程 + `GetSharedHandle`/`OpenSharedResource` + `IDXGIKeyedMutex` 帧同步 |
| BASELINE 全量重取证 | 8 脚本 × 多轮 | 所有阀门脚本需适配多窗口，oracle 图像需重新采集 |
| 迁移/双轨维护 | 不确定 | 0.x API 若直接改签名 = 不兼容升级 2.0；若双轨并行 = 维护两份渲染后端 |
| 总行数 | **2500-3000+ 行** | 不含测试/取证 |

**「难度是否更小」的结构对比**：

- 现状 = 在已知且已验证的单画布渲染管线上加 3 个功能点（位图挂载、事件转发、焦点），总计 ~330 行新代码 + 2 个取证脚本
- 窗口化 = 拆掉已验证的渲染管线（单 swapchain→N swapchain、单 Commit→N Commit、单金字塔→共享金字塔+同步），在 20 个窗口上重新证明正确性（8 个阀门脚本全量重跑、9 条不变量重审、BASELINE 重取证）
- 工程风险：窗口化涉及 DDA 共享句柄（未经本项目验证的技术路径）、KeyedMutex 死锁风险、DWM 合成时序依赖；现状路径不引入任何新的平台不确定性

**结论：窗口化的难度是现状的 7-10 倍（按代码量），且引入了至少 3 个未经本项目验证的技术不确定项。**

---

### 6. 混合方案 C（认真评估）

**方案 C 定义**：画布继续做视觉（维持单 swapchain / 单 Present / 单 Commit / 共享金字塔）；仅对「要当完整窗口用的块」可选升级为真 HWND 模式；或者反过来——承认库边界「库只做玻璃，窗口本体永远归宿主」。

**方案 C 的子变体评估**：

**C1：「可选升级为真 HWND」**——同一渲染器两种宿主模式。
- 否决理由：双宿主 = 两份渲染后端（画布路径 + per-HWND 路径），维护成本 > 纯画布或纯 HWND 的任一种；且「哪些块升级」的判断逻辑本身就需输入路由/焦点管理的全部复杂性

**C2（推荐）：「库只做玻璃，窗口本体永远归宿主」**——**这就是当前路线。**
- P0-2 输入路由：转发事件给宿主 sink，由宿主决定如何消费（`GlassInputSink` trait）
- P0-1 内容层：宿主推位图到玻璃表面（`ContentStore`）
- P1 焦点路由：宿主调用 `set_focus(block_id)` → 库通过 `SetFocus` 把焦点给宿主壳窗口 → 宿主壳窗口分发键盘/IME 给对应块
- 库的边界到此为止：键盘/IME/UIA/拖放/DWM 动画/Alt-Tab 全部归宿主壳窗口

**C2 的优点**：
- 零架构变更，所有现有不变量、取证脚本、门禁体系原样保留
- 宿主获得的能力：位图挂载点 + 事件转发 + 焦点协调——正是 ikun 分类框需要的最小集
- 天花板清晰可文档化：库不提供 X（键盘/IME/UIA/DnD/DWM 阴影），因为那是宿主壳窗口的责任

---

### 7. P0-2+P0-1+P1 之后宿主仍然自己做不到的事

| 能力 | 库是否提供 | 替代方案 |
|---|---|---|
| 键盘路由（IME/TAB/快捷键/Enter） | **否** | 宿主壳窗口 `WM_KEYDOWN` → 应用层决定发给哪个块 |
| UIA 无障碍（Narrator/屏幕阅读） | **否** | 宿主壳窗口注册 `IAccPropServer`；库仅提供块几何查询 API 给宿主 |
| 拖放注册（`RegisterDragDrop`） | **否** | 宿主壳窗口注册 `IDropTarget`；`WS_EX_TRANSPARENT` 逐帧切换带来一帧竞态（已有文档化降级预案） |
| DWM 窗口阴影/圆角动画 | **否** | 宿主壳窗口是真正的 `WS_OVERLAPPED`，自带 DWM 装饰 |
| Alt-Tab 缩略图 / 任务栏条目 | **否** | 宿主壳窗口提供 |
| Z-order 管理（`SetWindowPos`） | **否** | 宿主壳窗口管理 |
| 系统菜单 / 右键标题栏 | **否** | 宿主壳窗口提供 |

**天花板判断**：「仿真窗口」路线做不到的是真 HWND 天然提供的系统集成（Alt-Tab、任务栏、DWM 装饰、UIA）。但这些恰好是**宿主壳窗口的责任**——如果宿主没有壳窗口，那它本来就没有窗口（库也不应该替它造一个）。如果宿主有壳窗口，那这些能力已经有了，库只需要把像素和事件「嫁接」上去。

---

## 推荐路线与理由

**推荐：保持当前架构，完成 P0-2+P0-1+P1+P2 路线图，承认库边界「库只做玻璃，窗口本体永远归宿主」。**

理由：
1. **剩余工作量极小**（~80 行业务逻辑 + 验证），对比窗口化重写（2500-3000+ 行）
2. **零架构风险**：不引入新的平台不确定性（DDA 共享句柄、KeyedMutex、跨设备 DComp）
3. **性能基线不变**：所有现有阀门指标原样保留，不重定义不变量
4. **能力完备**：宿主获得的三件事（位图挂载、事件转发、焦点协调）正好覆盖 ikun 分类框场景
5. **天花板清晰**：文档化列出库不提供的 7 项能力及其替代方案，宿主可以据此判断是否适用

---

### 哪些已落地资产可复用

| 资产 | 复用方式 |
|---|---|
| P0-2 输入路由状态机（`core/routing.rs`，~250 行） | **不变**——按压仲裁在画布级做，与窗口数无关 |
| `GlassInputSink` trait（`api/event.rs`） | **不变**——同步回调契约已是现有模式 |
| P0-1 `ContentStore`（`gfx/content_layer.rs`） | **不变**——每块一张纹理，渲染侧无额外 GPU 调用 |
| 8 个取证脚本 | **全部复用**——画布不变、RT 不变、对比口径不变 |
| 性能阀门 `gate_perf.py` | **全部复用** |
| BASELINE 9+3 条不变量 | **全部成立**——无需重定义 |
| `cmp_visual.py` 跨版本四段对比 | **全部复用** |

---

## 外部引用

- MSDN `ID3D11Device::OpenSharedResource`: 跨设备纹理共享需 `D3D11_RESOURCE_MISC_SHARED` 标志
- MSDN DXGI Flip Model: flip model 下 DWM 直接从 swapchain buffer 合成，无额外 blit
- NVIDIA Advanced API Performance: Swap Chains: swapchain buffer 数量与延迟控制
- MSDN `IDCompositionDevice` / visual tree: 子视觉对象可由不同 `IDCompositionDevice` 创建，各自 Commit 独立生效
- MSDN `SetWindowDisplayAffinity` + `WDA_EXCLUDEFROMCAPTURE`: 排除窗口被 DDA/WGC/PrintWindow 采集
- CEF offscreen rendering + D3D11 共享纹理实践 — 验证跨进程 GPU 纹理共享链路可行性

## 不确定项

1. DDA `AcquireNextFrame` 返回的 `ID3D11Texture2D` 是否支持 `GetSharedHandle` 导出共享句柄——**未经本项目实测**，属于关键风险点
2. `IDXGIKeyedMutex` 在 DDA 采集帧率（~60 Hz）与渲染帧率（~120 Hz）不匹配时的行为和性能影响——未测试
3. N 个 DComp visual（不同 device）在同一树中的合成性能与 timing——MSDN 确认功能可行，但本项目无实测数据
4. `WS_EX_NOACTIVATE` 去掉后焦点抖动对宿主输入的实际影响——取决于宿主是用 `SetFocus` 还是 `WM_ACTIVATE` 管理焦点
