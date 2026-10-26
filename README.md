# liquid-verre-rs

Windows 桌面**玻璃窗口组件库**：上层应用创建一个玻璃窗口、配置它的外观，再把自己的 UI
叠在上面，就能得到「液态玻璃弹窗」这类效果。原生 **WGC / DDA 采集 + D3D11 模糊金字塔 +
DirectComposition 合成**，不依赖任何 WebView / 前端运行时。**多显示器**：每块屏独立采集、
独立金字塔、独立画布，分辨率与 DPI 各不相同也各自正确。

- crate 名：`liquid-verre-rs`　lib 名：`liquid_verre`　版本：`0.4.0`　许可：MIT
- 平台：Windows x64，依赖 DXGI Desktop Duplication、Windows.Graphics.Capture、DirectComposition 与 Per-Monitor V2 DPI 感知 <!-- TODO(verify): 源码/manifest 未声明最低 Windows 版本号，需确认后再写具体版本 --> ；`rust-version = 1.77`，edition 2021
- 依赖：`windows 0.58`、`serde`、`log`（**不**自带日志后端）

## 两种模式

| | `GlassMode::Liquid`（液态玻璃，默认） | `GlassMode::Transparent`（透明玻璃） |
|---|---|---|
| 参数 | `LiquidParams`，**37 个字段**（折射 / 色散 / 模糊 / 染色 / 光照 / 焦散 / 鼠标点光 / 阴影 / 拖拽滞后） | `TransparentParams`，**只有 3 个**：`corner_radius` / `opacity` / `color` |
| 管线 | 桌面采集 → 模糊金字塔（Pass A）→ Snell 折射 + RGB 色散 + Fresnel 高光 + 焦散 + 鼠标点光 → Pass B → DComp | 着色器走**早退分支**，不采样任何纹理（capture / frost / env / LUT 全不碰） |
| 成本 | 需要桌面采集与金字塔 | 整屏都是透明块时 **Pass A 整条跳过**（`FrameInput::needs_backdrop == false`），启动也不等采集就绪 |
| 阴影 | 有（`shadow_margin` 决定窗口区域与绘制矩形的外扩量） | 无（`shadow_margin()` 恒 0） |
| 适用 | 需要「桌面被折射 / 磨砂」观感的主视觉面板 | 弹窗底板、提示条、加载遮罩、大量并列小卡片 |

两种模式可以在**运行期**互换：`GlassWindow::set_style` / `set_liquid` / `set_transparent`。
同一个 `Engine` 里两种模式的窗口可以并存（`playground` 默认就会额外建一对 liquid / transparent 对照窗口，`--no-demo-pair` 可关）。

## 安装

```toml
# 本地 path 依赖
[dependencies]
liquid-verre-rs = { path = "../liquid-verre-rs/crates/liquid-verre" }

# 或 git 依赖（建议钉 tag / rev，别钉 branch：branch 会随上游漂移）
[dependencies]
liquid-verre-rs = { git = "https://github.com/baiwusanyu-c/tarui-glass.git", tag = "v0.4.1" }
# 对应 tag 还没推出来时先钉 commit（当前仓库只有 baseline/pre-refactor 一个 tag）：
# liquid-verre-rs = { git = "https://github.com/baiwusanyu-c/tarui-glass.git", rev = "<commit sha>" }
```

```rust
use liquid_verre as lv;   // lib 名是 liquid_verre（连字符的 crate 名不能直接当标识符）
```

三种消费方式（git 依赖 / crates.io / GitHub Release 产物）与完整发版流程见下文「发布与分发」。

## 最小可运行示例

```rust,no_run
use liquid_verre::{Engine, EngineConfig, GlassStyle, WindowConfig, WindowLevel};

let mut engine = Engine::new(EngineConfig {
    window_level: WindowLevel::ClickToFront,
    ..Default::default()
})?;
engine.start()?;                       // 起渲染线程（非阻塞）

let popup = engine.create_window(WindowConfig {
    rect: [320.0, 200.0, 560.0, 360.0],           // [x, y, w, h]，逻辑像素
    style: Some(GlassStyle::transparent(28.0, 0.40, [1.0, 1.0, 1.0, 1.0])),
    ..Default::default()
})?;

for ev in engine.poll_events() {
    println!("{ev:?}");
}
popup.set_corner_radius(40.0);         // setter 异步，延迟 ≤1 帧
engine.shutdown();
# Ok::<(), liquid_verre::Error>(())
```

「宿主就是一个玻璃演示程序」时可以直接阻塞跑：

```rust,no_run
let mut engine = Engine::spawn(EngineConfig::default())?;   // = new + start
let liquid = engine.create_window(liquid_verre::WindowConfig {
    rect: [900.0, 200.0, 640.0, 420.0],
    ..Default::default()
})?;
liquid.bring_to_front();
engine.shutdown();
# Ok::<(), liquid_verre::Error>(())
```

## API 速查

### `Engine`（一个进程通常只需要一个；`Drop` 自动 `shutdown`）

| 方法 | 签名要点 |
|---|---|
| `new(config: EngineConfig) -> Result<Self>` | 只建配置，**不**起线程；顺带装默认 stderr logger（上层已装则不覆盖）并打一行 `[boot]` 构建摘要 |
| `spawn(config: EngineConfig) -> Result<Self>` | = `new` + `start` |
| `start(&mut self) -> Result<()>` | 起渲染线程（幂等）。线程内做 `RoInitialize` + `SetProcessDpiAwarenessContext(PerMonitorV2)`；**DPI 感知是进程级**的，宿主已设过别的级别时这里只 `[warn]` 不失败 |
| `run(&mut self) -> Result<()>` | = `start` + `join`（阻塞到退出） |
| `join(&mut self) -> Result<()>` | 等线程结束，并把线程内的致命错误返回 |
| `shutdown(&mut self)` | 请求退出并 join，幂等 |
| `is_running() -> bool` | |
| `take_fatal_error() -> Option<Error>` | 非阻塞取走致命错误（取出即清空） |
| `create_window(WindowConfig) -> Result<GlassWindow>` | id 在调用侧预分配，句柄**立刻**可用；实际创建发生在下一帧。尺寸为 NaN / ≤0 ⇒ `Err(ErrorKind::InvalidConfig)`（**先校验原始输入再 sanitize**） |
| `window(id: u32) -> GlassWindow` | 廉价 `Arc` 克隆，不做存在性检查 |
| `windows() -> Vec<GlassWindow>` | 来自快照，最多滞后一帧 |
| `window_snapshots() -> Vec<WindowSnapshot>` | |
| `clear_windows()` | 销毁全部窗口 |
| `set_default_style(GlassStyle)` / `default_style() -> GlassStyle` | 设置会刷进所有 `inherits_style` 的窗口 |
| `set_window_level(WindowLevel)` / `window_level() -> WindowLevel` | 作用于**画布窗口** |
| `set_default_resize(ResizeOptions)` | 只影响**新建**窗口 |
| `monitors() -> Vec<MonitorSnapshot>` | |
| `snapshot() -> Snapshot` | 引擎整体状态 |
| `quality_level() -> u8` | 自适应质量档位（0 = 不降档） |
| `poll_events() -> Vec<EngineEvent>` | 批量取事件（队列上限 `EVENT_QUEUE_CAP = 256`） |
| `set_event_handler(Option<EventHandler>)` | 在**渲染线程**同步回调；`None` 取消 |
| `request_render()` | 手动请求一帧。任何 setter 都会自动置位，只有「改了库看不见的东西」时才需要 |
| `config() -> &EngineConfig` | 返回 sanitize 之后的配置 |

### `GlassWindow`（`Clone + Send + Sync`；窗口销毁后句柄不失效，setter 变 no-op、getter 返回 `None`）

- 读：`id()`、`is_alive()`、`snapshot() -> Option<WindowSnapshot>`、`rect()`、`size()`、
  `position()`、`style()`、`mode()`、`monitor_index()`、`canvas_handle() -> Option<isize>`、
  `engine_running()`
- 写（几何）：`set_rect(Rect)`、`set_size(w, h)`、`set_position(x, y)`
- 写（外观）：`set_style(Option<GlassStyle>)`（`None` = 恢复继承引擎默认）、
  `set_liquid(Option<LiquidParams>)`（`None` = 沿用当前参数）、
  `set_transparent(corner_radius, opacity, color)`、`set_corner_radius(f32)`、
  `set_opacity(f32)`、`set_color([f32;4])`
- 写（交互 / 层级 / 可见性）：`set_resize_options(ResizeOptions)`、`set_resizable(bool)`、
  `set_z_order(ZOrder)`、`bring_to_front()`、`send_to_back()`、`set_visible(bool)`、
  `set_interactive(bool)`、`set_movable(bool)`、`close()`
- 写（输入路由）：`set_input_sink(Option<Arc<dyn GlassInputSink>>)`（`None` = 摘除，行为回到库默认）
- 写（内容层）：`update_content(w_phys, h_phys, &[u8], Option<[u32;4]>)`、`clear_content()`

`set_opacity` / `set_color` 在透明模式改 `opacity` / `color`，在液态模式改 `base_alpha` / `tint_color`。
`canvas_handle()` 是给上层「把自己的 UI 窗口按同样矩形贴在玻璃之上」用的；
**不要**对它调 `DestroyWindow` / `SetWindowPos`，层级请走 `Engine::set_window_level`。

### 配置类型（全部 `Default` + serde `camelCase`，可直接用 JSON/TOML 喂）

| 类型 | 字段 |
|---|---|
| `EngineConfig` | `default_style`、`window_level`、`default_resize`、`capture`、`adaptive_quality`(默认 `true`)、`occlusion_culling`(默认 `true`)、`fullscreen_hide`(默认 `true`)、`fps_cap: Option<u32>`、`monitor_limit: Option<usize>`、`log_level`、`diagnostics` |
| `WindowConfig` | `rect: Rect`、`style: Option<GlassStyle>`(`None` = 继承默认)、`resize`、`z_order: Option<ZOrder>`、`interactive`(默认 `true`)、`movable`(默认 `true`)、`visible`(默认 `true`)；默认 rect = `[64, 64, 600, 400]` |
| `GlassStyle` | 枚举：`Liquid(LiquidParams)` / `Transparent(TransparentParams)`；方法 `mode()` / `needs_backdrop()` / `corner_radius()` / `shadow_margin()` / `sanitize()` / `scaled(dpi)` / `to_params()` / `liquid(p)` / `transparent(r, a, rgba)` |
| `TransparentParams` | `corner_radius`(默认 32)、`opacity`(默认 0.35)、`color`(默认 `[1,1,1,1]`) |
| `LiquidParams` | 37 个字段，默认值即本项目的视觉基线（见「默认参数」一节）；方法 `sanitize()` / `scaled(dpi)` / `inert()` |
| `ResizeOptions` | `enabled`、`edges`、`corners`、`grip_px`(默认 10)、`min_width`(24)、`min_height`(24)、`feedback`；方法 `edges_active()` / `corners_active()`（总开关关掉则两者都失效） |
| `ResizeFeedbackOptions` | `enabled`、`border_width`(默认 **5**，逻辑像素；`sanitize` 夹到 0.5–32)、`alpha`(默认 **1.0**)、`color`(`[1,1,1]`)。⚠ 0.4.1 起从 `3 / 0.95` 加粗，原因见「边缘 / 对角缩放」一节 |
| `WindowLevel` | `AlwaysOnTop`(**默认**) / `Normal` / `ClickToFront` / `AlwaysOnBottom`；`is_topmost()` / `raises_on_click()` |
| `ZOrder` | `Front` / `Back` / `Absolute(i32)` / `Relative(i32)`（绝对序号会被库整体归一化，**不保证数值长期稳定**） |
| `CaptureConfig` | `backend`、`retry_backoff_ms`(500)、`idle_keepalive_ms`(250)、`startup_wait_ms`(6000) |
| `CaptureBackend` | `Auto`(**默认**：HDR→WGC，SDR→DDA 优先) / `ForceDda` / `ForceWgc` |
| `DiagnosticsConfig` | `default_window`(默认 `false`)、`metrics`、`force_full`、`verify_validity`、`no_adaptive` |
| `LogLevel` | `Error` / `Warn` / `Info`(**默认**) / `Debug` / `Trace`；有 `From<LogLevel> for log::LevelFilter` |
| `Rect` | `pub type Rect = [f32; 4]` = `[x, y, width, height]` |

### 内容层（`update_content`）——每块一张位图

块本身只有玻璃。内容层让宿主把**预乘 BGRA8 位图**贴到块上：画在「本块玻璃之上、缩放 border 反馈
之下」，被本块有效圆角掩码裁剪；z 序交错绘制保证上层块的玻璃盖住下层块的内容（`verify_content.py`
对这一点做了判别力取证）。

```rust
// 物理像素；行距 = w_phys * 4；dirty = 块内半开区间 [x0, y0, x1, y1)，None = 全量。
win.update_content(800, 600, &premultiplied_bgra8, Some([16, 16, 116, 116]));
// 尺寸变化/隐藏/回收前先清除，避免旧内容按新矩形残留：
win.clear_content();
```

语义与成本：

- **1:1 不缩放**：纹理贴块矩形左上角，超出部分裁剪、不足部分透明；resize 后请 `clear_content`
  再按新尺寸推流（取证断言 5 证明无拉伸残留）。
- **静态零成本**：同内容不重传不重画；`dirty` 只上传子区（`UpdateSubresource` 子矩形），
  子区外像素逐位不动（取证断言 4）。
- **无内容的块/帧零开销**：内容存储为空时连查找都没有，渲染 pass 数与 0.4.2 完全一致——
  这是本库的第一门禁（无内容构建 vs 0.4.2 基线像素零差异，已实测）。
- 异步消费：与所有 setter 一致，提交后 ≤1 帧可见。
- 参数校验（长度 ≠ w·h·4、单边 > 8192）⇒ 丢弃该条 + warn-once，**不返回 Err**；
  w/h = 0 等价 `clear_content`。显存 = 每内容块 w·h·4（+ WDDM 镜像开销）。
- 文本/图标/矢量请宿主自己渲染进位图（D2D/GDI 均可）；库只吃位图，不内置文字渲染。

### 三个正交的交互开关（别混）

| 开关 | 关掉后的效果 | 仍然生效的部分 |
|---|---|---|
| `WindowConfig.interactive` / `set_interactive(false)` | **完全点击穿透**：不参与任何命中测试（`Manager::hit_test` 与 `hit_resize` 都过滤掉它），鼠标消息交给下层窗口 | 照样画出来（窗口区域仍包含它），`set_rect` 等 API 照旧可用 |
| `WindowConfig.movable` / `set_movable(false)` | 按住块**内部**不开始移动会话：不 `SetCapture`、不发 `MoveStarted`（`platform/window.rs::on_button_down` 直接早退） | **缩放把手仍然有效**（`hit_resize` 只看 `interactive && visible && resize.enabled`，不看 `movable`）；点击抬层仍然生效（同层 `bring_to_front` 与 `WindowLevel::ClickToFront`） |
| `ResizeOptions.enabled` / `set_resizable(false)` | 缩放把手不参与命中测试（`hit_resize` 跳过该块），也就没有悬停 border 反馈 | 整块拖动照旧（除非同时 `movable = false`） |

「不能挪位置」「不能改大小」「不接鼠标」是三件独立的事，分别由 `movable`、
`resize.enabled`、`interactive` 控制，两种玻璃模式共用同一套开关。
`movable = false` 只对**能被命中**的块有意义（`interactive = false` 时它根本按不住）。
两个边界行为：

- 正在移动中的块被 `set_movable(false)`：移动会话**立刻结束**（`runtime/commands.rs` 显式丢弃
  `DragSession::Move` 并清 `press`），否则它会继续跟着光标跑。缩放会话不受影响。
- `movable` 不参与渲染，因此 `apply_one` 对 `SetMovable` 返回 `false`（不脏化任何绘制账本），
  只置 `snapshot_dirty` —— 玻璃像素一个都不变，但下一帧快照里能读到新值。

### 输入路由（`GlassInputSink`）——把块当窗口用

块不是 HWND，鼠标消息只落到画布 wnd_proc。sink 就是库补上的那件「最小事件分发」：
光标在块内时，指针事件**同步**路由给你的实现，`Down` 的返回值决定库这次按压开不开移动会话。

```rust
use liquid_verre::{GlassInputSink, InputDecision, PointerEvent, PointerPhase};
use std::sync::Arc;

struct MySink;
impl GlassInputSink for MySink {
    fn on_pointer(&self, id: u32, ev: &PointerEvent) -> InputDecision {
        match ev.phase {
            PointerPhase::Down => {
                if in_icon_area(ev.block_local) { InputDecision::Consumed }
                else { InputDecision::Ignored }
            }
            PointerPhase::DoubleClick => { open_item(id, ev.block_local); InputDecision::Consumed }
            PointerPhase::Wheel => { scroll(id, ev.wheel_delta); InputDecision::Consumed }
            _ => InputDecision::Ignored,
        }
    }
    fn cursor_override(&self, _id: u32) -> Option<isize> { None }
}

win.set_input_sink(Some(Arc::new(MySink)));
```

硬规则（宿主交互正确性的契约，库行为以此为准，`tools/verify_input_sink.py` 逐条取证）：

1. **缩放把手带（含块外扩 grip）优先级恒高于 sink**：命中带内一律走库缩放会话 + hover
   border 反馈，**不询问 sink**。
2. 左键 `Down`（块内）先询问 sink：`Consumed` = 不开移动会话、**不 SetCapture**（OLE 拖出
   因此天然可用——`DoDragDrop` 请锚在宿主自己的壳窗口上）；`DragMove` = 开移动会话；
   `Ignored` = 库默认（等价 `DragMove`）。
3. `Down` 之后的同一按压序列**粘滞**：Move/Up 绑定转发给 owner 块（Up 必回按压时消费的
   块，即使光标已经离开）。
4. 未按压时 Move / DoubleClick / RightDown / RightUp（`Up(Some(Right))`）/ Wheel 转发给
   光标所在最上层块（与命中测试同一 z 序）。
5. 未设置 sink 的块与旧版本行为完全一致；`wheel_delta` = 滚轮格数（一格 ±1），
   `block_local` 是块内物理像素坐标（原点 = 块矩形左上角，不含阴影外扩）。
6. 回调在**渲染线程**（画布 wnd_proc 上下文）同步执行：微秒级返回、不阻塞、不重入引擎；
   **禁止在回调内调 `DoDragDrop`**（OLE 模态泵会卡死渲染线程、玻璃停绘）。回调内调库 setter
   是安全的（异步入队，≤1 帧生效）。

键盘不路由（画布 `WS_EX_NOACTIVATE`）：宿主用隐藏壳窗口接收键盘并做 OLE 拖出锚点。

### 把 IDropTarget 挂到画布（P1-2 拖入命中与 pump 稳定性契约）

画布 `WS_EX_TRANSPARENT` 逐帧切换使 `WindowFromPoint` 在无交互时穿透画布，
因此 OLE `IDropTarget` 不应直接注册到画布 HWND。推荐模式：

#### 壳窗口锚点模式（推荐）

宿主创建一个不可见的最小壳窗口（`WS_EX_TOOLWINDOW`，尺寸 1x1，无任务栏按钮），
将 `IDropTarget` 注册到该壳窗口上。拖拽命中判定由壳窗口 `DragEnter` 回调中
按屏幕坐标 ↔ 块内坐标换算，自己决定本次拖拽属于哪块：

```
// 屏幕坐标 → 块内逻辑像素坐标
fn screen_to_block(screen_pt: POINTL, block_rect: Rect, scale: f32) -> [f32; 2] {
    let block_logical_x = (screen_pt.x as f32 - block_rect[0]) / scale;
    let block_logical_y = (screen_pt.y as f32 - block_rect[1]) / scale;
    [block_logical_x, block_logical_y]
}
```

块矩形与 scale 从 `WindowSnapshot` 读取（`GlassWindow::snapshot()`）。
多块场景按 z 序从高到低命中第一个包含该点的块。

#### 命中/pump 稳定性承诺（基于实测）

`tools/verify_focus_drop.py` 对 `WS_EX_TRANSPARENT` 逐帧切换下 `WindowFromPoint`
命中率进行了 >= 50 轮实测（块内 9 个采样点 × 每点 10 次 × 每轮，间隔 >= 50ms，
模拟 OLE drag-over 反复探测时序）：

- **块内命中率 > 95%**：首次命中延迟 < 0.5ms（TRANSPARENT 摘除由主循环 idle tick
  驱动，无自锁死），稳态命中 120/120
- **块外穿透率 > 50%**（反向对照）：证明 `WS_EX_TRANSPARENT` 切换在正确起效，
  块外区域正确穿透到下层窗口
- **稳定漏检 = 0**：未出现同点连续 >= 3 次全 miss 的情况

因此 OLE `IDropTarget` 性能语义为：drag-over 反复探测可以稳定命中画布，
TRANSPARENT 摘除延迟在亚毫秒级。

#### 焦点路由（`set_focus_hwnd` → `EngineEvent::FocusHint`）

块不是 HWND，画布按设计 `WS_EX_NOACTIVATE`——**库永不代宿主触碰键盘焦点**。
2026-10-26 实测裁决：跨线程代调 `SetFocus` 没有安全方式——`AttachThreadInput`
对「没有消息泵的目标线程」会把调用方挂死（实测：渲染线程当场冻结、取证心跳
停摆），目标进程非前台时同样被系统策略拒。于是焦点时机以事件送达、焦点动作
归还宿主：

```
win.set_focus_hwnd(host_shell_hwnd as isize);  // 登记宿主壳窗口，0 = 清除
```

块被按压且 sink 返回 `Consumed` 时，库发出 `EngineEvent::FocusHint { id, hwnd }`
（`poll_events` 与 `set_event_handler` 两条消费路都送）。宿主两条路真正拿到焦点：

**best-effort 语义**：按压本就在画布上发生，前台合法性高、attach 窗口极短。
若 `ret=NULL`（例：目标进程非前台且 attach 被系统策略拒），库不重试、
不报错，仅打一次 `[warn]`。

**宿主兜底**（当库的 `SetFocus` 因 Win32 策略无法送达时）：
在 `GlassInputSink::on_pointer` 的 Down 回调里 `PostMessage` 给自己的壳窗口
（`PostMessage` 任意线程合法），壳窗口在自己线程调用 `SetFocus` 自己：

```rust
// sink 回调内（渲染线程，对宿主壳窗口发消息）
unsafe { PostMessageW(host_shell_hwnd, WM_USER_FOCUS, 0, 0); }
// 宿主壳窗口 wnd_proc 内（自己的线程）
match msg { WM_USER_FOCUS => { SetFocus(host_shell_hwnd); } }
```

宿主自己的线程在 `poll_events` 里收到 `FocusHint`，对壳窗口 `SetFocus`——壳窗口
本就属于宿主线程，同线程调用合法必成（playground 的 demo 即此实现）；
或在 sink 的 `Down` 回调里 `PostMessage` 给自己的壳窗口（PostMessage 任意线程
合法、无锁），壳窗口收到消息后自己 `SetFocus`。

守卫：画布自身 HWND 传进来会被拒绝登记（warn-once）——画布永不经任何路径获得
键盘，因此画布上的 ESC→关停引擎调试路径恒不可达。

`[event] FocusHint` + `[verify] focus_applied id= target=0x.. ret=0x..` 是
`verify_focus_drop.py` 的证据链（含按压后心跳连续=防冻结回归、连击 100 压力）。

#### 交叉引用

- sink 回调内**禁止 `DoDragDrop`**（OLE 模态泵会卡死渲染线程，玻璃停绘）
  → 见 §输入路由 契约第 4 条
- `set_focus_hwnd`：按压消费时发 `FocusHint` 事件，焦点由宿主在自己线程执行
  → 见 `EngineEvent::FocusHint` rustdoc（含 attach 挂死教训）
- 焦点守卫：画布自身 HWND 被传给 `set_focus_hwnd` 时拒绝登记 + warn-once
  → 见 `.trellis/tasks/10-23-focus-drop-routing/research/esc-and-focus-guard.md`

### 事件与错误

- `EngineEvent`（`#[non_exhaustive]`，19 个变体）：`Ready{monitor_count}`、
  `MonitorsChanged{monitor_count}`、`WindowCreated{id}`、`WindowDestroyed{id}`、
FocusHint{id,hwnd}、  `MoveStarted{id}`、`MoveEnded{id,rect}`、`ResizeStarted{id,handle}`、`ResizeEnded{id,rect}`、
  `LevelApplied{level}`、`CaptureBackendSelected{monitor_index,monitor_name,backend}`、
  `CaptureLost{monitor_index,reason}`、`CaptureRestored{monitor_index}`、
  `ExclusiveFullscreen{monitor_index,active}`、`QualityLevelChanged{level}`、
  `Warning{kind,message}`、`Fatal{kind,message}`、`EventsDropped{count}`、`Shutdown`。
  `ResizeHandle` = 8 个方向（`Left/Right/Top/Bottom/TopLeft/TopRight/BottomLeft/BottomRight`，
  `Display` 输出 kebab-case）。
  两种消费方式可并用：`set_event_handler`（渲染线程同步回调，**不得阻塞、不得重入引擎**）
  与 `poll_events`（有界队列 `EVENT_QUEUE_CAP = 256`，溢出丢最旧并在下次轮询补一条 `EventsDropped`）。
- `Error` / `ErrorKind`（`#[non_exhaustive]`，10 类）/ `Result<T>`：
  `GraphicsInit`、`ShaderCompile`、`MonitorEnumeration`、`WindowCreation`、`CaptureUnavailable`、
  `CaptureLost`、`InvalidConfig`、`NotSupported`、`Shutdown`、`Internal`。
  `Error::kind()` / `message()` / `with_kind()`；`Display` 输出 `[kind] message[: source]`；
  底层 HRESULT 保留在 `source()`。
- `BuildInfo` / `BUILD_INFO`：`version`、`git_sha`、`git_dirty`、`built_at_utc`、`target_triple`、
  `rustc_version`、`profile`、`shaders`（`(文件名, sha256 前 12 位)`，按文件名升序）；
  `BuildInfo::summary()` 是单行摘要，适合打进崩溃上报。由 `build.rs` 生成（零第三方依赖）。
- `install_default_logger()` / `install_default_logger_with_level(level) -> bool`。

### 快照类型

- `Snapshot`：`windows`、`monitors`、`default_style`、`window_level`、`default_resize`、
  `quality_level`、`fps`、`refresh_hz`、`on_battery`、`epoch`（每次发布递增）。
- `WindowSnapshot`：`id`、`rect`、`style`、`mode`、`resize`、`z_order`、`inherits_style`、
  `monitor_index`、`scale`、`visible`、`interactive`、`movable`、`hovered`、`moving`、`resizing`。
- `MonitorSnapshot`：`name`(`\\.\DISPLAY1`)、`origin`、`size`、`dpi`、`scale`、`primary`、
  `capture_backend`(`"dda"` / `"wgc"` / `"none"`)、`canvas_visible`、`canvas_handle`。

## 宿主集成样例（把块当窗口用的最小闭环）

以「桌面分类框」这类场景为例，四项核心交互在库能力上的完整拼法（Rust 示意，
类型均出自 `liquid_verre::api`）：

```rust
use liquid_verre::{Engine, EngineEvent, GlassInputSink, GlassWindow, InputDecision,
                  PointerEvent, PointerPhase};
use std::sync::Arc;

// ① 内容推流：宿主（D2D/GDI/任何渲染器）把图标+标签画进预乘 BGRA8 位图
struct IconSink { /* 线程安全的图标命中表快照 */ }
impl GlassInputSink for IconSink {
    fn on_pointer(&self, id: u32, ev: &PointerEvent) -> InputDecision {
        match ev.phase {
            PointerPhase::Down =>
                if self.hit_icon(ev.block_local) {
                    // 选中态变化 → 只需重画受影响子区；顺带唤焦点（见 ④）
                    self.post_focus_to_shell();   // PostMessage 到自己壳窗口，无锁安全
                    InputDecision::Consumed
                } else { InputDecision::Ignored } // 空白区 → 库开移动会话
            PointerPhase::DoubleClick => { self.open_selected(id); InputDecision::Consumed }
            PointerPhase::RightDown => { self.queue_context_menu(id, ev.block_local); InputDecision::Consumed }
            PointerPhase::Wheel => { self.scroll(id, ev.wheel_delta); InputDecision::Consumed }
            _ => InputDecision::Ignored,
        }
    }
    fn cursor_override(&self, _id: u32) -> Option<isize> { self.hover_cursor() }
}

win.set_input_sink(Some(Arc::new(IconSink::snapshots())));
win.update_content(w_px, h_px, &frame, Some([x0, y0, x1, y1]));  // 仅状态变化时推 dirty
win.set_focus_hwnd(shell_hwnd as isize);                          // 键盘可达（FocusHint 驱动）
```

② **拖出文件**：`Consumed` 按压库不抓鼠标（不 SetCapture），宿主在壳窗口上起
`DoDragDrop`（OLE 锚点=壳窗口，**绝不在 sink 回调内调**）。
③ **拖入文件**：`IDropTarget` 注册到 `canvas_handle()`（每屏一个），`Drop` 坐标按块矩形
自行换算路由——命中稳定性由 `verify_focus_drop.py` 取证背书（见「把 IDropTarget 挂到画布」节）。
④ **键盘**：`FocusHint` 事件 → 宿主在自己线程 `SetFocus` 壳窗口（ESC/Delete/F2 到宿主
自己的消息循环，库零参与焦点）。

移动/缩放/叠放/固定/DPI 全部沿用库既有能力（`movable`/`ResizeOptions`/`ZOrder`/`set_interactive`），
不需要为「窗口本体」再创建任何 HWND。

## 层级语义（两个不同的东西，别混）

- **画布层级 `WindowLevel`**：作用于**画布窗口**（每块显示器一个全屏 `WS_EX_NOREDIRECTIONBITMAP`
  popup），即「整层玻璃」相对**其它应用窗口**的位置。
  - `AlwaysOnTop` → `WS_EX_TOPMOST` + `HWND_TOPMOST`（**默认**，与重构前一致）
  - `Normal` → `HWND_NOTOPMOST`，随其它窗口正常遮挡
  - `ClickToFront` → `Normal` + 点击命中玻璃块时 `HWND_TOP`（抬到普通窗口层最前，**不抢焦点**）
  - `AlwaysOnBottom` → `HWND_BOTTOM`，在所有普通窗口之下（仍在桌面之上，否则玻璃不可见）
  - 一律带 `SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE`：画布带 `WS_EX_NOACTIVATE`，
    抬升绝不能把焦点从宿主抢走；层级操作也不该触碰几何（否则白白让 DWM 重新裁剪）。
  - ⚠ **遮挡剔除只在非置顶层级有意义**：`AlwaysOnTop`（默认）时画布在所有普通窗口之上，
    普通窗口**盖不住**玻璃，`EnumWindows` 扫到自家画布就终止 ⇒ 遮挡者集合恒空。
    `Normal` / `ClickToFront` / `AlwaysOnBottom` 下才会有块被完整覆盖，此时被覆盖的块
    不再驱动桌面采集、也不再计入光标影响盒（`EngineConfig.occlusion_culling`，默认开；
    `GLASS_OCCLUSION=0` 关）。剔除的是**触发源**而不是绘制：像素照常画，
    遮挡移开后 ≤125 ms 内恢复（那一小段玻璃里是旧桌面，恢复帧会取新桌面并整屏重画）。
- **块 z 序 `ZOrder`**：同一画布**内**玻璃块之间的叠放，用 `set_z_order` / `bring_to_front` /
  `send_to_back`。`Manager` 按需整体归一化序号，因此 `Absolute(n)` 的 `n` 不保证长期稳定。

## 线程模型与一致性契约

- 库内**只有一个渲染线程**：消息泵、桌面采集、Pass A（模糊金字塔）、Pass B（逐块绘制）、
  `Present` + DComp `Commit` 全在它上面。这是「20+ 块不卡顿」与「静止桌面 CPU≈0」的前提。
- **写**：`GlassWindow` 的 setter 把命令推进队列并置渲染请求位；渲染线程在**帧首一次性**消费。
  因此 setter 是**异步**的，延迟 ≤1 帧；一帧内连发 100 条 `set_style` 只有最后一条生效，
  不会渲染 100 次。
- **读**：渲染线程在状态真的变化时发布 `Snapshot`（否则 4 Hz 兜底），getter 从快照读，
  因此是**最终一致**（≤1 帧）。换来的是读路径不与渲染线程抢锁。
- **不允许**从上层线程直接触碰 GPU 资源或内部 `Manager`；一律走命令队列 + 快照。
- `GlassWindow` 是 `Clone + Send + Sync`，可以在上层任意线程持有与调用。
- `Engine` 不是 `Send`（持有 `JoinHandle`）；需要跨线程共享时自己包 `Arc<Engine>`
  （`playground` 就是这么做的：面板线程持 `Arc<Engine>`，主线程轮询事件）。
- `EngineEvent` 回调在**渲染线程**执行：不要阻塞、不要重入引擎（会死锁），重活转到自己的线程。
- `GlassInputSink::on_pointer` / `cursor_override` 同样在**渲染线程**（画布 wnd_proc 上下文）
  同步回调：契约与 `EventHandler` 一致，另加一条硬禁令——**回调内禁 `DoDragDrop`**
  （OLE 模态泵卡渲染线程）。回调内调 setter 安全（异步入队）。

## 日志与错误捕获

- 库内一律用 [`log`](https://crates.io/crates/log) facade，**不自带日志后端**。
  `Engine::new()` 会顺带装一个可选的默认 stderr logger（行格式与重构前的命令行版本逐字节一致：
  `[boot] / [info] / [warn] / [error] / [verify] / [metrics] / [adaptive]`）。
- **想接管全部日志**：在 `Engine::new()` **之前**调用 `log::set_logger(...)`
  （`env_logger` / `tracing-log` / 自己的日志面板都行）。之后 `install_default_logger()`
  会返回 `false` 且不产生任何影响。
- 级别过滤走 `EngineConfig::log_level`（默认 `Info`）；上层已装 logger 时以对方的过滤器为准。
- 错误：只有**不可恢复的初始化失败**才是 `Err`（`Engine::new/start/run/join/create_window`）。
  采集失效、独占全屏让路、DPI 感知被宿主占用、句柄指向已销毁窗口这类**可恢复**情况
  一律走 `EngineEvent::Warning` / 专用事件，不会让方法返回 `Err`。
  渲染线程内的致命错误 → `[error]` + `EngineEvent::Fatal` + `Engine::take_fatal_error()`。
- ⚠ `[metrics]` / `[adaptive]` / `[verify]` 前缀与字段名是 `tools/` 质量阀门的**解析接口**，
  自定义 logger 时请保留它们（见 `.trellis/spec/backend/logging-guidelines.md`）。

## 坐标系与量纲

- `Rect` / 一切长度参数 = **逻辑像素 = 主屏物理像素**，坐标是**虚拟屏绝对坐标**（多屏通用）。
- 进程声明 Per-Monitor V2；块落到 `scale = dpi / 主屏 dpi` 的屏上时，库按该比例换算
  「像素量纲」的字段（`corner_radius` / `bevel_width` / `thickness` / `blur_max` / `blur_min` /
  `mouse_radius` / `mouse_spread` / `rim_width` / `shadow_margin`），**逻辑尺寸不变**、
  物理观感一致。主屏 `scale` 恒 1.0，逐字段原样返回，视觉零变化。
- 尺寸夹紧：`MIN_SIZE = 24.0`、`MAX_SIZE = 8000.0`；位置允许「半出屏」（弹窗滑入动画），
  但挡住 `±1e9` 这类把 f32 精度吃光的值。
- 一切来自上层的数值都先过 `sanitize()` 才进 GPU；NaN / ±inf 用 `clamp_finite`
  （NaN→下界、+inf→上界、-inf→下界）—— `f32::clamp` 遇 NaN 会**原样返回 NaN**，
  而 NaN 一进 GPU，SDF 会整块消失且不报任何错。
- 有效圆角 = `clamp(corner_radius * scale, 0, 短边 / 2)`（`Block::effective_radius`），
  **统一由样式派生**，不再有「每块各存一份半径」。
## 构建 / 检查 / 打包 / 阀门

这是一个 Cargo workspace（`crates/liquid-verre` + `playground` + `xtask`）。
两条等价路径，改一边就要改另一边：

```powershell
# 路径 A：cargo xtask（.cargo/config.toml 里 alias 到 run --package xtask --release --）
cargo xtask build                          # cargo build --workspace --release（--debug 可切档）
cargo xtask check                          # fmt --check + clippy -D warnings + test --release
cargo xtask package [--out DIR] [--no-zip] # 收集产物到 dist/ + build-info.json + zip
cargo xtask gate                           # 先 build，再 python tools/gate_perf.py（透传退出码）
cargo xtask clean                          # cargo clean + 删 dist/

# 路径 B：Node 脚本（零 npm 依赖，含自实现 zip；跨平台、固定 UTF-8）
node scripts/build.mjs [--debug] [--only <pkg>]
node scripts/check.mjs
node scripts/gate.mjs
node scripts/package.mjs [--out DIR] [--no-zip] [--debug]
node scripts/zip.mjs <sourceDir> <outZip>
node scripts/release.mjs --bump patch --dry-run   # 本地发版助手（不 commit/tag/push，见「发布与分发」）
```

单独跑：

```powershell
cargo test --release --workspace                                    # 功能阀门（库 136 + 集成 11 + playground 5 + doctest 2 = 154）
cargo clippy --workspace --all-targets --release -- -D warnings
cargo fmt --all -- --check
python tools/gate_perf.py                                           # 性能硬门槛（FAIL 退出码 1）
cargo run --release --bin liquid-verre-playground -- --port 8787    # dev 控制面板
```

`cargo xtask package` / `node scripts/package.mjs` 的产物（`dist/liquid-verre-rs-<ver>-<triple>/`）：
`lib/libliquid_verre.rlib`（若开启对应 crate-type 还会有 `liquid_verre.dll` / `libliquid_verre.a`）、
`bin/liquid-verre-playground.exe`、`shaders/*.hlsl`、`README.md`、`LICENSE`、`CHANGELOG.md`、
`build-info.json`（name / version / target / gitSha / gitDirty / builtAtUtc / profile / artifacts）。

## 发布与分发

### 作为库被别的项目消费（按推荐顺序）

**① Git 依赖 —— 现在就能用，不需要任何发布动作**

```toml
[dependencies]
liquid-verre-rs = { git = "https://github.com/baiwusanyu-c/tarui-glass.git", tag = "v0.4.1" }
```

- package 名是 **`liquid-verre-rs`**，lib 名是 **`liquid_verre`**：`Cargo.toml` 里写连字符的那个，
  代码里 `use liquid_verre::{Engine, EngineConfig, WindowConfig};`。
- **钉 tag 或 rev，不要钉 branch**：branch 会随上游漂移，锁不住「我这次构建用的是哪份代码」。
  对应 tag 还没推出来时先用 `rev = "<commit sha>"`。
- **仅 Windows 可用**（D3D11 + DXGI Desktop Duplication / WGC + DirectComposition）。
  跨平台宿主要写 target 限定依赖，否则非 Windows 上直接编译失败：

```toml
[target.'cfg(windows)'.dependencies]
liquid-verre-rs = { git = "https://github.com/baiwusanyu-c/tarui-glass.git", tag = "v0.4.1" }
```

**② crates.io —— 一次性配好凭据后，由 tag 流水线自动发布**

```toml
[dependencies]
liquid-verre-rs = "0.4"
```

crate 名 `liquid-verre-rs` 目前在 crates.io 上**还没被占用**（2026-10-21 查 `/api/v1/crates/liquid-verre-rs`
返回 404 = 可注册）。要让别的项目能 `cargo add liquid-verre-rs`，需要三步：

1. **首发必须手工做一次**（crate 在 crates.io 上还不存在时，流水线与 trusted publishing 都无从谈起）：
   到 <https://crates.io/settings/tokens> 生成 API token（scope 勾 `publish-new` + `publish-update`），
   本地 `cargo login`（token 落进 `%USERPROFILE%\.cargo\credentials.toml`，只需一次），
   再 `node scripts/release.mjs --bump patch --publish`（或去掉 `--publish`，改由 CI 发）。
2. **之后自动发布，二选一**：
   - **token 路线（现成可用）**：仓库 Settings → Secrets and variables → Actions →
     New repository secret，名字必须正好是 `CARGO_REGISTRY_TOKEN`。推 `v*` tag 后
     `release.yml` 的 `crates-io` job 会 `cargo publish -p liquid-verre-rs --no-verify`；
     **没配这个 secret 就跳过并在日志里留一条 notice** —— 只出 GitHub Release、靠 git 依赖分发
     也是完全可用的方式。token 建议设过期时间，泄露时在同一页 revoke。
     ⚠ 判定写在 step 里而不是 job 级 `if:`：**secret 不能直接用在 `if:` 条件里**
     （GitHub 官方安全指南原话 "Secrets cannot be directly referenced in if: conditionals"）。
   - **trusted publishing 路线（免长期 token，crates.io 2025 起支持 GitHub Actions OIDC）**：
     首发完成后，在 crate 页面 Settings → Trusted publishers 填 publisher `baiwusanyu-c`、
     repository `tarui-glass`、workflow `release.yml`、environment 留空；再给 `crates-io` job 加
     `permissions: id-token: write`，publish 命令换成
     `cargo publish -p liquid-verre-rs --no-verify --trusted-publishing=always`
     （需要较新的 stable cargo，先 `cargo --version` 确认认得这个旗标）。
   - 两条路线**别同时开**：本地 `--publish` 之后 CI 再发同一版本会因「已存在」失败
     （GitHub Release 不受影响，只是多一个红叉）。
3. 跨平台宿主同样写 `[target.'cfg(windows)'.dependencies]`。

**③ GitHub Release 的 zip —— 给人看的产物 + 溯源，不是给 Cargo 用的**

`dist/liquid-verre-rs-<ver>-<triple>.zip`（由 `scripts/package.mjs` 生成、`release.yml` 上传）内容：
`lib/libliquid_verre.rlib`、`bin/liquid-verre-playground.exe`、`shaders/*.hlsl`、
`README.md` / `LICENSE` / `CHANGELOG.md`，以及 `build-info.json`
（name / version / target / profile / gitSha / gitDirty / builtAtUtc / node / 逐产物字节数）。

⚠ **`lib/libliquid_verre.rlib` 不能被 Cargo 直接引用**：rlib 不是稳定 ABI，
它绑定编译它的 rustc 版本与 crate 元数据哈希，换个工具链就加载不了。
zip 的定位是「**给人看的产物 + `build-info.json` 溯源**」；机器消费请走 ① 或 ②。
若要给**非 Rust 宿主**（C / C# / Node 原生插件）用，才需要额外开
`crate-type = ["cdylib"]` 并手写 C 头 —— **当前未做，是待决策项**
（`package.mjs` 已经会在 `liquid_verre.dll` / `libliquid_verre.a` 存在时顺手打进 zip，
所以打开 crate-type 不需要改打包脚本）。

### 发版流程

**本地发版（`scripts/release.mjs`）**：

```powershell
node scripts/release.mjs --bump patch --dry-run          # 先看会做什么：不落盘、不动 git
node scripts/release.mjs --bump patch --ship             # 一键发版：闸门 + 改版本号/CHANGELOG + commit + tag + push
node scripts/release.mjs --bump patch --ship --no-push   # 同上，但 push 留给人复核
```

不带 `--ship` 时脚本只落盘「版本号 + CHANGELOG 标题」，然后打印要人工敲的三条 git 命令
（项目约定：提交必须由人显式发起）；`--ship` 就是那句显式授权，tag 一推上去 `release.yml` 立刻跑。

`release.mjs` 依次做：① **工作区必须干净**（否则发出去的版本对不上 commit）；
② 算目标版本（`--bump patch|minor|major` 或 `--version x.y.z`，tag 名默认 `v<version>`，
`--tag` 可覆盖，但必须匹配 `v` + semver —— 否则不触发 `release.yml` 的 `tags: ["v*"]`）；
③ **git 预检**：detached HEAD / tag 本地或远端重名 / 落后上游 / 没有 origin，一律在耗时的闸门**之前**失败；
④ **CHANGELOG 预检**：`--ship` 时必须有 `## [x.y.z-unreleased]` 标题，否则拒绝发布（release notes 会是空的）；
⑤ `node scripts/check.mjs`（= fmt + clippy `-D warnings` + test）；
⑥ `cargo package -p liquid-verre-rs --list --allow-dirty` 干跑（验 readme / license /
路径依赖等可发布性）；⑦ 改根 `Cargo.toml` 的 `[workspace.package] version`，并把 CHANGELOG 的
`## [x.y.z-unreleased]` 标题替换成 `## [x.y.z] - YYYY-MM-DD`；
⑧ `--ship` 时 `git add -A` + `git commit -m "release: x.y.z"`（`--message` 可覆盖）+ 附注 tag
+ **一条命令同时推分支与 tag**，最后打印 Actions / Release 链接；
⑨ `--publish` 才会在本地直接 `cargo publish`（需先 `cargo login`），排在 push **之前** ——
失败时 tag 还没出去，脚本会打印回滚命令（`git tag -d <tag>` + `git reset --hard HEAD~1`）。

**推 tag 之后（`.github/workflows/release.yml`）**：

1. `windows-latest`：`node scripts/check.mjs` → `node scripts/package.mjs` → 上传 `dist/` artifact；
2. `ubuntu-latest`：从 `CHANGELOG.md` 里抽出 `## [x.y.z]` 那一段当 release notes，建 GitHub Release
   并挂 `dist/*.zip` + `dist/*/build-info.json`（版本号含 `-` 时自动标 prerelease）；
3. 若配了 secret `CARGO_REGISTRY_TOKEN`：`cargo publish -p liquid-verre-rs --no-verify`（token 有无在 step 里判，没配则跳过 + 一条 notice；secret 不能写在 job 级 `if:` 里）。

也可以不推 tag：Actions 页面手动触发 `release`（`workflow_dispatch`，可填 tag 名；留空则只构建不发布）。

**每次 push / PR（`.github/workflows/ci.yml`）**：`cargo fmt --all -- --check` +
`cargo clippy --workspace --all-targets --release -- -D warnings` + `cargo test --release --workspace` +
`cargo package -p liquid-verre-rs --list` 干跑。**刻意不跑**像素取证（`cmp_visual` / `verify_*` /
`repro_tremble`）与性能阀门（`gate_perf`）：它们依赖 DDA 桌面复制、真实 vsync 节拍与可交互的桌面会话，
CI 的无交互会话里测出来的数字没有意义（口径见 `.trellis/spec/backend/measurement-and-gates.md §1`）。
**CI 绿只代表「编译 / 单测 / 静态检查 / crate 可打包」**，不代表视觉与性能没回归。

### 发布前需要提供什么

| 项 | 用在哪 | 现状 |
|---|---|---|
| GitHub 仓库 URL | `crates/liquid-verre/Cargo.toml` 的 `repository`（缺失时 `cargo package` 会告警）+ 本节与「安装」一节的 git 依赖片段 | **已填** `repository = "https://github.com/baiwusanyu-c/tarui-glass"`（`cargo package --list` 已无告警）。仓库若改名为 `liquid-verre-rs`，这里 + 本节与「安装」一节的 git 依赖片段要一起改 |
| 是否开 crates.io | 决定要不要配 Actions secret `CARGO_REGISTRY_TOKEN`（或改用 trusted publishing） | **未配**：不配 = 只出 GitHub Release，git 依赖照常用。开的话首发必须本地 `cargo login` + `--publish` 手工做一次，详见上文 ② |
| tag 命名约定 | `release.yml` 的触发条件是 `tags: ["v*"]`，`release.mjs` 默认生成 `v<version>` | **`v` + semver**（例 `v0.4.1`）；`baseline/pre-refactor` 这类非 `v*` tag 不会触发发布 |
| crate 内 README | `cargo package` 要求 `readme` 指向 **crate 目录内**的文件（workspace 根的 README 不会被打进 `.crate`） | 已有 `crates/liquid-verre/README.md`（短版 + 指回仓库根） |

## dev 控制面板（playground）

`playground/` **不是库的业务代码**：它只用 `liquid_verre` 的公共 API 驱动引擎，
没有一个 `pub(crate)` 后门，因此同时兼作 API 的活文档。重构前它是 `src/ui/`
（tiny_http 混在渲染依赖图里）。

```powershell
cargo run --release --bin liquid-verre-playground -- --port 8787 --transparent --level click
```

| 参数 | 作用 |
|---|---|
| `--port <n>` | 面板端口（默认 8787，监听 `127.0.0.1`，自动打开系统浏览器） |
| `--no-browser` | 不自动打开浏览器 |
| `--transparent` | 默认样式用透明玻璃（`GlassStyle::transparent(28.0, 0.38, [1,1,1,1])`） |
| `--no-demo-pair` | 不额外创建 liquid / transparent 对照窗口 |
| `--no-default-window` | 不创建居中默认玻璃 |
| `--level <top\|normal\|click\|bottom>` | 画布层级（默认 `top`） |

HTTP 接口：`GET /`、`GET /api/schema`、`GET /api/state`、`GET /api/events`、
`POST /api/style`、`POST /api/mode`、`POST /api/resize`、`POST /api/window`、`POST /api/level`。
`POST /api/window` 的 `action`：`create` / `close` / `clear` / `front` / `back` / `rect` /
**`movable`**（`{"action":"movable","id":<id>,"value":<bool>}` → `GlassWindow::set_movable`）。
窗口区有一个「可拖拽移动」按钮切换它，文案随状态在「可拖拽移动 / 禁止拖拽移动」之间变；
`GET /api/state` 的每个窗口都回显 `movable` 字段。

`playground/src/schema.rs` 是面板控件的**单一来源**（`liquid_fields()` 37 项、
`transparent_fields()` 3 项、`resize_fields()` 10 项），前端按它动态生成控件。
两个方向的单测都钉死了：`LiquidParams` 里有字段没进面板 ⇒ `every_param_is_exposed_in_the_panel` FAIL；
面板里有 key 但 `LiquidParams` 没有 ⇒ `every_liquid_field_exists_in_params` FAIL。
**给库加参数时必须同步加一行 schema。**

取证 / 计量模式（`GLASS_MANY` / `GLASS_FAR_GRID` / `GLASS_SPREAD` / `GLASS_DEFAULT_BLOCK` /
`GLASS_INTERNAL_SHOT` / `GLASS_SHOT_EVERY_MS` / `GLASS_METRICS` / `GLASS_SIM_BACKDROP` /
`GLASS_VERIFY_PASS_A` / `GLASS_VERIFY_VALIDITY`）下 playground **不会**加演示窗口 ——
多出来的窗口会让逐位比对图整体错位（实测抓到过 single 场景 interior 差 49885 像素）。

### 输入路由 demo（取证用）

`GLASS_INPUT_SINK_DEMO=1` 给默认块挂 demo sink（左半 `Consumed` / 右半 `DragMove`），
打 `[verify]` 事件日志（含按压瞬间的 capture 归属）并起 400ms 的 rect 心跳线程，
供 `tools/verify_input_sink.py` 驱动与判定；默认不开，不影响任何基线取证。

### 内容层 demo（取证用）

`GLASS_CONTENT_DEMO=<场景>`（checker/solid/dirty/clear_seq/stack/resize_* 等 6+ 场景）让默认块
推确定性图案内容并打 `[verify]` 日志，供 `tools/verify_content.py` 驱动；默认不开，不影响任何基线取证。

## 目录结构

```
liquid-verre-rs/                     # Cargo workspace（根 Cargo.toml 只有 [workspace]）
├─ Cargo.toml                        # members / workspace.package(0.4.0, edition 2021, rust 1.77, MIT)
│                                    #   / workspace.dependencies(windows 0.58, serde, serde_json, tiny_http, log)
│                                    #   / [profile.release] opt-level=3, lto=fat, codegen-units=1,
│                                    #     panic=abort, strip=symbols（与重构前逐字一致，别改）
├─ .cargo/config.toml                # [alias] xtask
├─ .github/workflows/                # ci.yml（push/PR：fmt + clippy -D warnings + test --release + package 干跑）
│                                    # release.yml（v* tag / 手动：check + package → GitHub Release → 可选 crates.io）
├─ crates/liquid-verre/              # ★ 库本体（package liquid-verre-rs，lib liquid_verre）
│  ├─ Cargo.toml                     # readme = "README.md"（cargo package 要求 readme 在 crate 目录内）；
│  │                                 #   repository 仍是注释占位，等仓库 URL 定下来再填
│  ├─ README.md                      # crate 自带短 README（会进 .crate 包），指回仓库根的完整文档
│  ├─ build.rs                       # 生成 OUT_DIR/build_info.rs（自实现 SHA-256 + UTC 时间戳，零依赖，绝不 panic）
│  ├─ shaders/                       # glass_vs / glass_ps / blur_vs / blur_down / blur_up / env_ema /
│  │                                 #   border_vs / border_ps（缩放反馈 border 的独立叠加 pass；include_str! 内嵌）
│  └─ src/
│     ├─ lib.rs                      # crate 门面：#![warn(missing_docs)] + 全部对外 pub use
│     ├─ error.rs  logging.rs  build_info.rs
│     ├─ api/                        # ★★ 唯一公共面
│     │  ├─ config.rs                # EngineConfig / WindowConfig / GlassStyle / GlassMode / TransparentParams /
│     │  │                           #   ResizeOptions / ResizeFeedbackOptions / WindowLevel / ZOrder /
│     │  │                           #   CaptureConfig / CaptureBackend / DiagnosticsConfig / LogLevel / Rect
│     │  ├─ engine.rs                # Engine
│     │  ├─ handle.rs                # GlassWindow + Command 队列 + Shared + Snapshot/WindowSnapshot/MonitorSnapshot
│     │  └─ event.rs                 # EngineEvent / EventHandler / ResizeHandle / EVENT_QUEUE_CAP
│     ├─ core/                       # ★ 纯逻辑：零 windows:: 引用、零 unsafe，因此全部可单测
│     │  ├─ params.rs                # LiquidParams(37 字段) + Profile + Shape + sanitize/scaled/inert
│     │  ├─ block.rs                 # Block + MIN_SIZE/MAX_SIZE + clamp + effective_radius
│     │  ├─ manager.rs               # 块集合 / z 序 / hit_test / hit_resize / 跨屏归属
│     │  ├─ handle.rs                # ResizeEdges(8 方向) + hit_handle(先角后边) + to_shader_edges/corners
│     │  ├─ draw.rs                  # DrawBlock(投影块，Copy) + ResizeFeedback + draw_rect
│     │  ├─ style.rs                 # StyleCache（避免每帧重跑 sanitize）
│     │  ├─ geometry.rs              # ★ 矩形 / 影响盒运算的单一来源
│     │  ├─ quality.rs               # QualityCtrl（自适应质量 0–4 档）
│     │  └─ sdf.rs                   # 折射 LUT(256 项) + clamp_for_block + is_lod3
│     ├─ runtime/                    # 主循环（只在渲染线程存在，无锁）
│     │  ├─ mod.rs                   # run()：初始化 + 逐帧顺序 + 诊断布局 seed_diagnostic_blocks
│     │  ├─ app.rs                   # App 状态机 + FrameCtx + render_monitor + publish_snapshot + read_cursor
│     │  ├─ commands.rs              # Command 的帧首消费点
│     │  ├─ occlusion.rs             # 逐块遮挡剔除的**编排**（8 Hz）：扫哪块屏、结论写回谁、翻转沿日志
│     │  └─ monitor.rs               # MonitorRt（一屏一套）+ region_margin() + influence_px()
│     ├─ platform/                   # ★ Win32 集中地
│     │  ├─ window.rs                # 窗口类 LiquidVerreCanvas / 窗口过程 / WM_NCHITTEST / SetWindowRgn / 点击穿透
│     │  ├─ level.rs                 # WindowLevel → WS_EX_TOPMOST + SetWindowPos
│     │  ├─ display.rs               # 显示器枚举（物理像素 + 每屏 DPI）/ refresh_hz / on_battery / 独占全屏检测
│     │                              #   + dwm_cloaked（全仓唯一 DWM 调用点）/ window_desc（翻转沿打身份日志）
│     │  └─ occlusion.rs             # EnumWindows z 序扫描：本屏画布**之上**的不透明窗口矩形（关卡顺序 = 性能契约）
│     ├─ gfx/                        # D3D11 / DXGI / DComp
│     │  ├─ device.rs  shaders.rs  swapchain.rs  dcomp.rs
│     │  ├─ compositor.rs            # 每屏画布：脏区账本、金字塔有效性、Clear→PassA→PassB→Present→Commit、取证落盘
│     │  ├─ pyramid.rs               # Pass A：下采样×4 + env EMA + frost tent×4（scissor 幂等）
│     │  ├─ glass_renderer.rs        # Pass B：per-block cbuffer(c0..c18) + 逐块 LUT 缓存 + z 序绘制
│  │                            #   + border 叠加 pass（draw_borders，独立 BorderCb）
│     │  └─ gpu_timer.rs             # D3D11 timestamp（只用于离线归因，禁止进闭环判据）
│     ├─ capture/                    # mod.rs(后端选择 + 合成源) / dda.rs(脏矩形 + 局部拷贝) / wgc.rs(帧池)
│     ├─ input/                      # cursor.rs(GetCursorPos 轮询，零 hook) / drag.rs(拖拽 + 8 方向缩放状态机)
│     └─ diag/                       # env.rs(GLASS_* 唯一解析点) / metrics.rs([metrics] 行)
├─ playground/                       # dev 控制面板（bin liquid-verre-playground，publish = false）
│  └─ src/ main.rs  panel.rs  schema.rs  web/index.html
├─ xtask/src/main.rs                 # build / check / package / gate / clean（零第三方依赖）
├─ scripts/                          # _lib.mjs build.mjs check.mjs gate.mjs package.mjs zip.mjs
│                                    #   release.mjs（本地发版助手：改版本号 + CHANGELOG 标题；不 commit/tag/push）
├─ tools/                            # 性能探针与逐像素取证（Python）
├─ .trellis/spec/                    # 项目约定（backend/ + guides/）
├─ README.md  CHANGELOG.md  BASELINE.md  performance_report.md  LICENSE  AGENTS.md  CLAUDE.md
```

分层边界（完整版见 `.trellis/spec/backend/directory-structure.md`）：
`api/` 是唯一公共面；`core/` 不得依赖 `windows::`；`unsafe` 只在 `platform/` `gfx/` `capture/`
与 `runtime/` 的平台边界点位（`api/` 与 `core/` 必须保持 0 个）；上层不得直接触碰 GPU 资源。

## 渲染管线（一屏一帧）

```
PeekMessage 泵
→ 每屏采集 try_capture(wait)         # DDA 的阻塞等待 = 主循环唯一休眠点
→ 帧首一次性消费上层命令              # 一帧内连发 N 条同类 setter 只有最后一条生效
→ prep（参数/质量档/块投影/hover+把手/拖拽/窗口区域/点击穿透/影响盒/遮挡剔除 8 Hz/快照发布）
→ 渲染门控：触发源全空则整帧跳过（不 Clear、不 Pass A/B、不 Present、不 Commit）
→ 每屏：整屏 Clear → Pass A（按脏区 scissor；全透明屏整条跳过）→ Pass B（z 序画全部块）
→ Present(1) → DComp Commit（每屏一次）
→ QualityCtrl.tick(own_us)           # own_us = prep + PassA + PassB + Commit，不含采集等待与 Present 阻塞
```

| 关键点 | 实现 |
|---|---|
| 整屏只采集一次 | `capture/dda.rs`，成本 O(屏幕)，与块数无关 |
| **只搬变化的像素** | `GetFrameDirtyRects` / `GetFrameMoveRects` → 逐矩形 `CopySubresourceRegion`（>24 个退回整屏） |
| **模糊只算块看得见的地方** | `gfx/pyramid.rs`，Pass A 的 scissor = 桌面脏区 ⊕ 模糊核 ∩ 块足迹 ⊕ 采样余量 |
| **Pass B 每帧画全部块** | flip 模型 back buffer 在 Present 后内容不保，「只画脏块」会让没画的块整块消失 |
| 每块只画自己的矩形 | `gfx/glass_renderer.rs`，per-block viewport + scissor |
| **没有玻璃的屏整帧短路** | 不清屏、不 Pass B、不 Present、不 Commit |
| **被全屏应用压住的屏整屏停产** | `MonitorRt::hidden_fs` 同时是**停产门**：采集段与渲染段都跳过该屏（画布已 `SW_HIDE`，那些像素进不了用户眼睛）。恢复走「采集失效恢复」同一条路径（`invalidate()` + `ready=false`），**零陈旧帧** |
| **被完整遮挡的块不驱动采集** | `runtime/occlusion.rs` 8 Hz 扫 z 序；被不透明窗口完整盖住（判据用含阴影外扩的**绘制矩形**）的块不进 `wants_bd`、不进 `mouse_relevant`。**不改绘制集合** ⇒ 单帧像素逐位不变。⚠ 默认 `AlwaysOnTop` 下收益接近 0（置顶玻璃本来就不该被普通窗口遮住） |
| **全透明屏跳过 Pass A** | `FrameInput::needs_backdrop == false`（需求 F3.1 的性能来源） |
| **事件驱动** | 无触发源就整帧跳过；静止桌面不 Present、不 Commit |
| 自反馈根治 | `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`（DDA/WGC 均已验收：不是黑块） |
| **HDR 桌面** | HDR/宽色域输出下 DDA 复制面是 HDR 参考值（整张过曝）；自动改走 **WGC**，由 DWM 做 HDR→SDR。SDR 输出仍 DDA 优先；`GLASS_BACKEND` 可强制 |
| 拖拽期零 `SetWindowRgn` | 按下时一次性扩到整窗，抬起后收紧（旧实现每帧重建 HRGN，是 100~280 ms 长尾的主因） |
| 不抢焦点 | 画布带 `WS_EX_NOACTIVATE`，层级操作一律 `SWP_NOACTIVATE` |
| 独占全屏自动让路 | 4 Hz 轮询，命中就隐藏该屏画布 **并整屏停产**（采集 + 渲染全跳），退出后恢复。判据 = 前台窗口 `WS_POPUP` 且无 caption/thickframe 且铺满整块屏，外加三条身份排除（自家画布 / 无前台 / **`Progman` 桌面本身**）与 cloaked 否决；命中日志会打出触发窗口的 `class`/`title`/`rect`。`EngineConfig.fullscreen_hide`（默认 `true`）/ `GLASS_FS_HIDE=0` 可关 |
| 环境色不闪烁 | 1/16 层 GPU ping-pong EMA，τ=150 ms，零回读；**只在捕获内容真变化时推进** |
| **块与块之间零串扰** | 鼠标 border 渐变乘 `fall × gate(2.5R..3R)`；缩放反馈只作用于被 hover 的那一块；脏区影响盒 = `max(3R+32, 2*grip+16) + ceil(shadow_margin)`，与之严格对齐 ⇒ 盒外精确为 0 |
| 撑不住时自动降档 | 自适应质量控制器（**默认开启**）：持续丢 vblank 才逐级降，恢复后逐级升回；level 0 与未启用时**逐位一致** |

### 边缘 / 对角缩放（8 个把手）

- 判定唯一入口 `core/handle.rs::hit_handle(rect, cursor, grip)`：**先角后边**
  （同时落在左边带与上边带内 ⇒ 判为对角 `TOP_LEFT`，不是单边）。命中带伸出块外 `grip/2`，
  窗口区域同步外扩 `max_grip + 2`，否则那部分鼠标消息根本到不了我们窗口。
- `ResizeOptions { enabled, edges, corners, grip_px, min_width, min_height, feedback }`
  可分别开关边缘 / 对角；`edges_active()` / `corners_active()` 都受总开关约束。
- 悬停反馈是**加粗白色 border**（`ResizeFeedbackOptions`，默认宽 **5** 逻辑像素、alpha **1.0**、纯白；
  投影时 `DrawBlock::feedback.width = border_width * scale`，200% DPI 下就是 10 物理像素。
  0.4.1 之前是 `3 / 0.95` —— 在 100% DPI 屏上实测「看得见但不醒目」，用户验收时反馈触发不够明显；
  border 画在玻璃形状**内侧**（SDF 内距离 band），因此加粗**不影响**影响盒公式）：
  - **直边只画到圆角切点为止**（`winX/winY` 用 `hs - r` 做切点窗，`smoothstep` 软化，不拐弯）；
  - **对角沿圆弧拐弯**（用 SDF 内侧距离做 band，在圆角象限自然沿弧走）；
  - 两种掩码在**源头互斥**：`to_shader_edges()` 在对角时返回全 0，`to_shader_corners()`
    在单边时返回全 0（否则切点附近会叠出一个亮斑）；
  - 返回值再乘 `coverage`，保证 border 不漏到玻璃形状之外。
  实现在独立的 `shaders/border_ps.hlsl`（`GlassRenderer::draw_borders` 在 Pass B 之后叠加绘制），
  与液态着色完全解耦；直边到圆角切点为止、对角沿弧拐弯，两种掩码在 Rust 侧源头互斥。
- 缩放时**锚定不动边**：夹到最小尺寸时固定边纹丝不动（否则会出现「左边滑走」）。
- 事件：`ResizeStarted{id, handle}` / `ResizeEnded{id, rect}`；系统光标由 `ResizeEdges::cursor_id()`
  经 `WM_SETCURSOR` 设置。

### 自适应质量档位（默认开启，`EngineConfig.adaptive_quality` / `GLASS_ADAPTIVE=0` 关）

| level | 动作 | 实测收益 |
|---|---|---|
| 0 | 全部效果开启，与未启用控制器时**逐位一致** | 默认 |
| 1 | `frost` 保持 1/2 分辨率（省掉 Pass A 的整屏 tent 上采样） | +14% fps、−27.4 MB(单屏)/−35.5 MB(双屏)；模糊层差异 ≤3/255 |
| 2 | + 帧率上限 60（把抖动变成恒定 60 fps） | 消除 judder |
| 3 | + 关闭焦散与 Beer-Lambert 吸收 | 少量 ALU |
| 4 | + 关闭 RGB 色散（3 tap → 1 tap） | 省 2 次采样/像素 |

判据：1 秒窗口内「**我们自己的工作**（prep + Pass A + Pass B + Commit）> 1.25 × 目标帧间隔」
的渲染帧占比 > 25%（电池 15%）连续 2 次才降档；升档需连续 4 个干净窗口**且距上次降档 ≥20 s**，
升上去 10 s 内被打回视为失败探针，探针间隔翻倍（上限 300 s）。
**必须排除采集等待（实测 8~32 ms）与 Present 阻塞（实测 6.6~25 ms）**：
我们自己的工作只有 30~80 µs/帧。档位变化发 `EngineEvent::QualityLevelChanged`，并打 `[adaptive]` 日志。

## 默认参数（= 本项目的视觉基线，改动必须重做取证）

`LiquidParams::default()` 的两个「看起来像手误」的数字是**重构前那个二进制稳态运行时真正渲染出来的值**：

- `tint_opacity: 0.01`（不是设计值 0.018）
- `blur_min: 20.0 * 0.12`（不是设计值 3.0，而且**必须写成表达式**）

来历与逐位取证结论见 `CHANGELOG.md` 的 Fixed 一节与 `core/params.rs` 的注释。
其余默认值：`corner_radius 32`、`lens_coverage 0.15`、`bevel_width 26`、`ior 1.50`、`thickness 26`、
`refr_strength 1.0`、`dispersion 0.25`、`absorption 0.18`、`clarity 0.10`、`blur_max 20`、
`progressive_curve 1.6`、`edge_always 0.55`、`adaptive_tint 0.35`、`base_alpha 1.0`、
`tint_color [0.92,0.95,1.0,1.0]`、`env_tint_mix 0.25`、`light_dir [-0.45,-0.72,0.53]`、
`light_intensity 1.0`、`spec_intensity 0.85`、`spec_power 42`、`spec_spread 0.20`、
`fresnel_f0 0.14`、`rim_width 1.5`、`caustic_gain 0.55`、`mouse_radius 220`、
`mouse_intensity 1.35`、`mouse_spread 160`、`proximity_glow 0.35`、`shadow_opacity 0.10`、
`shadow_margin 24`、`lag_seconds 0.012`、`shape RoundedRect`、`super_n 4.0`、`profile Circular`。

## 性能指标

下表是**重构前**（单 bin crate `liquid-glass`）在本机实测的数字，作为回归基线保留。
本机：3120×2080@120 Hz/200% + 1920×1080@100%（Intel Ultra 9 285H / Arc 140T），
桌面持续重绘 47~110 次/秒。

| 指标 | 改造前（单屏，最初实现） | 重构前基线（单屏折算） | 重构后（`liquid-verre-rs`） |
|---|---|---|---|
| Pass A / 帧 | 6.49 Mpx，每帧必跑 | **0.79 Mpx**，最低 **0 次/帧** | <!-- TODO(verify): 重构后复测待填（python tools/perf_probe.py 的 pass_a_mpx_avg / pass_a）--> |
| Pass B 脏区 / 帧 | 6.49 Mpx（整屏） | **0.50 Mpx**（单块）/ 1.5~2.0 Mpx（20 块） | <!-- TODO(verify): 重构后复测待填（dirty_mpx_avg）--> |
| DDA 拷贝 / 帧 | 6.49 Mpx（整屏） | 1.7~4.0 Mpx（脏矩形） | <!-- TODO(verify): 重构后复测待填（copy_mpx_avg）--> |
| CPU render 均值 | 0.294 ms | **0.094~0.118 ms** | <!-- TODO(verify): 重构后复测待填（avg_render_ms）--> |
| CPU render 峰值 | 17~31 ms | **0.66~0.94 ms** | <!-- TODO(verify): 重构后复测待填（max_render_ms）--> |
| 常驻 Private | 182.7 MB | **128.4 MB** | <!-- TODO(verify): 重构后复测待填（tools/ab_memory.py）--> |
| 20 块静止 CPU | 8.83% 单核 | **5.32%** | <!-- TODO(verify): 重构后复测待填 --> |
| 20 块拖拽 CPU | 12.34% 单核 | **3.57~10.02%** | <!-- TODO(verify): 重构后复测待填 --> |
| 锁屏 / 采集失效 | **83.25%**（96 kHz 空转） | **0.469%** | <!-- TODO(verify): 重构后复测待填 --> |
| 单块拖拽 fps / p50 | — | 113~116 / 8.32~8.36 ms（120 Hz 锁相） | <!-- TODO(verify): 重构后复测待填（gate_perf single-drag）--> |
| 20 块拖拽 fps / p50（默认 lat=1） | — | 70~74 / 13.0~13.2 ms | <!-- TODO(verify): 重构后复测待填（gate_perf many-drag）--> |
| 20 块拖拽 fps / p50（吞吐档 `GLASS_FRAME_LATENCY=2`） | — | **105.0 / 8.34 ms** | <!-- TODO(verify): 重构后复测待填 --> |

> 重构后的数字**尚未复测**（库化改动本身不改渲染路径，但需要一次干净窗口的 A/B 才能下结论）。
> 复测方法见 `.trellis/spec/backend/measurement-and-gates.md`：轮转 A/B + ≥3 重复 + 中位数，
> 且必须满足机器健康判据 `rel_max < 5 ms`、`acq_max < 25 ms`，否则数据作废。

历史视觉零变化取证（重构前）：Pass A 重构前后 overlay **0 / 6,489,600 像素差异**；
脏矩形路径 vs 全量路径 **6/6 场景 0 差异**；局部拷贝 vs 整屏参考拷贝 **7 轮 840 帧 0 差异**；
多屏改造前后主屏 overlay **alpha 通道 0 差异**。详见 `performance_report.md`。
## `GLASS_*` 环境变量

**上层应用应当优先用 `EngineConfig` / `DiagnosticsConfig`**（结构化、可 serde）；
环境变量是给 `tools/` 脚本与命令行用的后门。名字保持 `GLASS_*` 不变 ——
`tools/` 下的回归脚本与 `gate_perf.py` 全部按这些名字取证，**改名等于打断质量阀门**。

解析点分两批，都**只在启动 / 构造期读一次**，不进热路径
（每帧 `env::var` = 一次进程环境锁 + 一次 `String` 分配）。

### A. 主循环批（`diag/env.rs::EnvOverrides::load()` 是唯一解析点）

| 变量 | 作用 |
|---|---|
| `GLASS_METRICS=1` | 每 **5 秒墙钟窗口**输出一行 `[metrics]`（CPU / 脏区 / Pass A / DDA / 帧率 / 分位数）。= `DiagnosticsConfig.metrics` |
| `GLASS_DEFAULT_BLOCK=1` | 启动即建一块**居中默认玻璃**（`w=clamp(主屏宽*0.62,720,1200)`、`h=clamp(主屏高*0.68,480,760)`）。**库默认不建任何窗口**（那是上层应用的职责），取证脚本必须显式打开。= `DiagnosticsConfig.default_window` |
| `GLASS_MANY=1` | 启动即铺 20 块密集级联布局（步长 `0.62w + 22` / `0.60h + 22`，与重构前一致，否则 `gate_perf` 的 many-drag 与历史数字不可比） |
| `GLASS_SPREAD=1` | 每块显示器正中各放一块（多屏 + DPI 验收） |
| `GLASS_FAR_GRID=1` | 4 块互不重叠、分开足够远的确定性布局（`verify_mouse_isolation` 的硬证明）。不能用 `GLASS_MANY` 代替：密集重叠会让邻近余晖沿半透明阴影逐块传递，判定集合的传递闭包吞掉全部 20 块 |
| `GLASS_EXTREME_SPEC=1` | 极端高光回归：`spec_intensity = 4.0`、`spec_power = 1.0`（验证 bevel/plane 与圆角转角不断裂） |
| `GLASS_SIM_BUSY=<ms>` | **拖拽帧**注入确定性忙等，用来确定性地驱动自适应控制器降档/回升 |
| `GLASS_SIM_BACKDROP=<W>x<H>@<HZ>` | 确定性模拟桌面脏区（固定放主屏正中）。**不能**用于「增量==全量」像素 oracle |
| `GLASS_MONITORS=<n>` | 只用前 n 块屏。= `EngineConfig.monitor_limit` |
| `GLASS_VERIFY_PASS_A=1` | 拍一张脏矩形受限帧后，同进程再渲一帧「全量 Pass A + 整屏 Pass B」，输出 `.full.bmp` 对照图 |
| `GLASS_INTERNAL_SHOT=<path>` | 取证截图输出路径（按屏加 `.m<i>.bmp`）。未同时设 `GLASS_SHOT_EVERY_MS` 时会**强制全量重画** |
| `GLASS_CURSOR_AT=x,y` | **全程钉死**光标（`App::pinned_cursor` → `App::read_cursor()`），不再只设初值。窗口过程与主循环都走 `read_cursor()`，因此命中测试与渲染用的是同一个光标 |
| `GLASS_ADAPTIVE=0` | 关闭自适应质量控制器（**默认开启**）。level 0 与关闭时逐位一致。= `DiagnosticsConfig.no_adaptive` |
| `GLASS_OCCLUSION=0` | 关闭逐块遮挡剔除（**默认开启**）。关掉后 `Block::occluded` 恒为 `false`、两个消费点短路 ⇒ 与未引入本机制**逐位一致**。= `EngineConfig.occlusion_culling` |
| `GLASS_FS_HIDE=0` | 关闭「独占全屏应用检测」（方案 R-16）的隐藏 + 整屏停产。= `EngineConfig.fullscreen_hide`（默认 `true`），两者取「与」。取证脚本必须设它，否则闸门不 hermetic：该检测的形状判据与「无边框最大化的普通应用」（Electron / Tauri 那一类）重合，命中即整屏停产 ⇒ 脚本拿到 0 帧连拍并整批假 FAIL。`tools/` 下所有像素/性能脚本都已注入，**唯一例外是 `verify_occlusion.py`**（它就是测这条门的） |

### B. 子系统批（只在自己模块的构造期读一次）

| 变量 | 读取点 | 作用 |
|---|---|---|
| `GLASS_BACKEND=dda\|wgc` | `capture/mod.rs` | 强制采集后端（默认 Auto：HDR 输出→WGC，SDR→DDA 优先、失败回退 WGC）。强制 dda 且拿到 FP16 复制面时打一次 `[warn]`（背景会过曝） |
| `GLASS_SYNTH_CAP=1` | `capture/mod.rs` | 用**确定性合成图案**覆盖采集结果，使跨进程 / 跨时间的逐像素取证可复现 |
| `GLASS_COPY_MODE=multi\|union\|full` | `capture/dda.rs` | 局部拷贝策略。`multi`(默认)=逐脏矩形 `CopySubresourceRegion`，超过 `MAX_DIRTY_RECTS = 24` 退回整屏；`union`=只拷脏矩形并集 bbox 一次；`full`=永远整屏 `CopyResource` |
| `GLASS_SIM_DIRTY=<w>x<h>@<n>` | `capture/dda.rs` | 用 n 个固定尺寸、确定性散布的脏矩形取代真实脏矩形，使三种拷贝策略在完全相同的搬运量下对比 |
| `GLASS_SKIP_COPY=1` / `GLASS_SKIP_DIRTY=1` | `capture/dda.rs` | 归因开关：跳过像素搬运 / 跳过脏矩形查询（画面会不正确，仅实验用） |
| `GLASS_VERIFY_COPY=1` | `capture/dda.rs` | 并行维护一张「永远整屏 `CopyResource`」的参考纹理，周期性回读并逐像素比对局部拷贝结果，自证脏矩形没漏区域 |
| `GLASS_WGC_BUFFERS=<2..8>` | `capture/wgc.rs` | WGC 帧池缓冲数（方案 12.7：>90 Hz 用 3，否则 2；下限夹到 2） |
| `GLASS_FRAME_LATENCY=<1..8>` | `gfx/device.rs` | DXGI 渲染队列深度，**默认 1**（跟手优先）。`=2` 为吞吐档：20 块拖拽 73.9→105.0 fps、p50 13.0→8.34 ms，代价 +1 帧呈现队列 ≈ +8 ms 光标到光子（快速拖拽时有「玻璃跟在光标后面」的拖尾感） |
| `GLASS_ENV_EMA=0` | `gfx/pyramid.rs` | 关闭环境色时域平滑（**默认开**，τ=150 ms），回到逐帧直采 1/16 层 |
| `GLASS_FROST_HALF=1` | `gfx/pyramid.rs` | 手动强制 `frost` 保持 1/2 分辨率（= 自适应控制器的 level 1）。省掉整屏 8-tap tent 上采样（约 1.0 个整屏当量 + 26 MB 常驻）。**开启后输出不再与默认档逐位一致**（差异 ≤3/255） |
| `GLASS_PASS_A_HZ=<hz>` | `gfx/compositor.rs` | Pass A 限频（自适应控制器的一档）。未设 / `0` = 不限频，此时与未引入本机制**逐位一致**。限频只影响 `frost` 与 1/16 环境色探针的新鲜度；清晰层、块几何、折射、色散、高光、鼠标点光、Pass B 仍每帧更新 |
| `GLASS_FORCE_FULL=1` | `gfx/compositor.rs` | 关闭脏矩形优化，每帧整屏重画（A/B 对照用）。= `DiagnosticsConfig.force_full` |
| `GLASS_VERIFY_VALIDITY=1` | `gfx/compositor.rs` | 漏更新自检：金字塔有效性虚报 / 该重画却漏掉的块。只打 `[verify]` 计数（≥1 秒节流），不改渲染。= `DiagnosticsConfig.verify_validity` |
| `GLASS_INTERNAL_SHOT_AT_MS=<ms>` | `gfx/compositor.rs` | 延迟到该时刻之后的第一帧再截图。**必须 > 0 才测得到脏矩形路径**（首帧本来就是整屏重画）。取**稳态帧**用 2500（> 环境色 EMA 的 τ=150 ms 收敛时间） |
| `GLASS_SHOT_EVERY_MS=<n>` | `gfx/compositor.rs` | 连拍模式，每 n ms 拍一张（序号进文件名）。连拍时**不**强制全量重画（保留增量路径），且默认只写 composed 一个文件 |
| `GLASS_BURST_OVERLAY=1` | `gfx/compositor.rs` | 连拍时额外写 `.overlay.bmp` / `.capture.bmp` / `.soft.bmp` / `.frost.bmp` |
| `GLASS_SKIP_PASS_A=1` / `GLASS_SKIP_PASS_B=1` / `GLASS_SKIP_PRESENT=1` / `GLASS_SKIP_CLEAR=1` | `gfx/compositor.rs` | 归因开关：分别跳过某一段 GPU 工作（画面会不正确，**仅实验用**，默认全关） |
| `GLASS_GPU_TIMINGS=1` | `gfx/compositor.rs` | 额外创建 D3D11 timestamp query。**会打断驱动的命令批处理、显著拖慢管线**（实测把帧率打到 15 fps），因此独立于 `GLASS_METRICS`，只在专门做 GPU 归因时开；**禁止进闭环判据** |

## 质量阀门

```powershell
cargo test --release --workspace        # 功能阀门：库 136 个单测 + 集成 11 + playground 5 个 schema 单测 + doctest 2 = 154
python tools/gate_perf.py               # 性能阀门：硬门槛不过退出码 1
```

`gate_perf.py` 硬门槛：单块拖拽 `fps ≥ 90` 且 `p50 ≤ 9.0 ms`；20 块默认档 `fps ≥ 60`；
20 块吞吐档（`lat=2` + `SIM_BACKDROP=900x700@120`）`fps ≥ 90`；三场景 `idle Private < 200 MB`。
软门槛（只 WARN）：20 块默认档 `fps ≥ 90`、拖拽 CPU `< 5%` 单核。

8 个回归脚本（重构前后都必须全绿）：

```powershell
python tools/verify_dirty.py             # 脏矩形渲染 vs 全量渲染，逐像素（含真实拖拽）；改 blur kernel/scissor 时连跑 ≥3 次
python tools/verify_mouse_isolation.py   # 拖一块，其余块必须 0 像素变化（含反向对照）
python tools/verify_validity.py          # 漏更新自检：虚报 / 漏画计数必须为 0
python tools/repro_tremble.py far        # 分离布局：拖一块，其他块逐帧不变（连拍取证）
python tools/repro_tremble.py many       # 密集 20 块同上
python tools/verify_adaptive.py          # 自适应控制器：降档 / 回升 / 显存回收闭环
python tools/ab_latency.py               # GLASS_FRAME_LATENCY 1 vs 2 轮转 A/B（吞吐档依据）
python tools/ab_memory.py                # 常驻内存受控 A/B（level 0 vs level 1）
```

行为取证（0.5.0 输入路由新增；改消息路由 / 会话收尾时必跑）：

```powershell
python tools/verify_input_sink.py        # GlassInputSink 五条行为断言：物理注入 + rect 心跳 + capture 归属
python tools/verify_content.py           # 内容层五条断言：精确采样 / 叠放交错 / clear 逐位回基线 / dirty 隔离 / resize 无残留
```

视觉改动额外：

```powershell
python tools/cmp_visual.py               # 跨版本稳态帧四段取证（deep/interior/ring/outside）
python tools/verify_crossscreen.py       # 跨屏拖拽 + DPI 换算验收
python tools/verify_env_ema.py           # 环境色 EMA 开/关 首帧等价
python tools/smoke_panel.py              # 面板 HTTP 冒烟：movable 开关 / feedback 默认值 / applyToAll 链路
python tools/shot_zoom.py <bmp> <png> <cx> <cy> [half]   # 取证图 4x 近邻裁剪放大（border 等细视觉验收用）
python tools/verify_all.py               # 上述 5 项一次跑完（含跨版本对比，需要先建 82a49e8 的 worktree）
python tools/attribute.py                # 拖拽帧时间归因：逐项关掉 Pass A / Pass B / Present
python tools/perf_probe.py --case all --seconds 12 --warmup 4 --out tools/final.json
python tools/measure_frost_half.py       # frost 半分辨率档的视觉代价 + 性能收益量化
python tools/ab_influence.py             # 光标影响盒修复前/后两个二进制的同环境 A/B
```

测量规则（本机结论的唯一合法来源）：轮转交错 A/B + ≥3 重复 + 中位数/p95；
机器健康判据 `rel_max < 5 ms`、`acq_max < 25 ms`，劣化窗口的数据一律作废；
视觉取证只能看 `GLASS_INTERNAL_SHOT` 的 composed 图（`WDA_EXCLUDEFROMCAPTURE` 使 OS 截屏看不到玻璃）。
详见 `.trellis/spec/backend/measurement-and-gates.md`。

## 不可破坏的 9 条不变量

完整版见 `BASELINE.md` 与 `.trellis/spec/backend/rendering-pipeline.md §5`：
① 每帧自包含（整屏清 + Pass B 画全部块，Pass A 按脏区 scissor）；
② 金字塔有效性不虚报（`!covered` 先于 `backdrop_changed`）；
③ 模糊链幂等（`DOWN_GROW=[14,12,8,0]`、tent `32/24/16/8`、`BLUR_REACH=96`、`HYSTERESIS=192`、
`pad = 128 + min(vel*max_lag,128)`）；
④ 环境色 EMA 只在捕获内容真变化时推进；
⑤ 一切光标驱动的视觉都是有限支撑窗，影响盒 `max(3R+32, 2*grip+16) + ceil(shadow_margin)`
必须严格覆盖（**含缩放反馈**；直边/对角掩码在源头互斥）；
⑥ 脏区三来源始终并集，不得短路；
⑦ HDR 输出自动切 WGC；
⑧ 自适应判据只看「我们自己的工作」；
⑨ 元数据与上层输入先 sanitize（`clamp_finite`，NaN 不进 GPU）。

## 基线与回滚点

- 重构前基线：tag `baseline/pre-refactor`、分支 `backup/pre-refactor-baseline`（= `82fc311`）；
  重构起点 `4dca041`（基线 + 阀门，无行为变化）。快照说明见 `BASELINE.md`。
- 过程证据与决策记录（含被否决方案）：`performance_report.md` 第一~十四轮。
- 变更清单：`CHANGELOG.md`（`0.4.0` = 库化重构，`0.4.1` = 自检轮修复 + `movable` + 发布流水线）。
- 项目约定（给 AI 协作者）：`AGENTS.md` / `CLAUDE.md`；工程规范：`.trellis/spec/`。

## 许可

MIT。见 `LICENSE`。