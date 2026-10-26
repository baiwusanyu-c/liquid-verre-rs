# liquid-verre-rs 能力补充清单（分类框「库窗口即本体」重构）

> 背景：ikun 分类框重构中，分类框的窗口本体将直接由 liquid-verre-rs 的 GlassWindow（画布块）承担，
> 不再为每个分类框创建独立 Win32 窗口。图标 / 栅格 / title / 固定等功能全部移植到库窗口上。
> 本文档 = 按项目实际功能盘点后，库需要补齐的能力清单与 API 建议（面向库作者）。
> 基线版本：liquid-verre-rs 0.4.2。

---

## 一、为什么必须补这些能力（一句话版）

Win32 的铁律：**鼠标/键盘消息只发给 HWND**。库的"玻璃窗口"不是窗口——每屏只有一个画布
HWND，块只是画布上的绘制矩形 + 库内部两条交互状态机（拖 / 缩放）。所以：

1. **绘制**：库 Pass B 只会画玻璃，宿主内容没有挂载点；
2. **输入**：画布 wnd_proc 目前只处理
   `WM_NCHITTEST / WM_LBUTTONDOWN / WM_LBUTTONUP / WM_MOUSEMOVE / WM_SETCURSOR`，
   右键、双击、滚轮、键盘消息直接被 DefWindowProc 吞掉，宿主收不到任何一个字节；
3. **焦点**：画布带 `WS_EX_NOACTIVATE`，键盘输入永远进不来。

类比浏览器 DOM：视觉上确实是「玻璃层在下、内容层在上、事件往下派发」，但 DOM 成立的前提是
引擎具备三样东西——**图层挂载点、事件分发、焦点系统**。库要成为"分类框窗口"，只需要把
这三样的最小版本补上：`update_content` / `input sink` / `set_focus_hwnd`。

---

## 二、按项目功能逐条映射

| 分类框功能 | 现在靠什么实现 | 新模型下的问题 | 库需要的能力 | 状态 |
|---|---|---|---|---|
| 玻璃背景/圆角/折射/阴影 | 自研 liquid_glass（已删） | — | — | ✅ 已有 |
| 图标、标签、title 文字、按钮的绘制 | 宿主窗口 DIB + ULW 提交 | 块上没有宿主像素的挂载点 | **P0-1 内容层** | ❌ |
| 图标点击/选中/Ctrl/Shift 多选 | WM_LBUTTONDOWN + grid_hit_test | 按压被画布收到 → 库默认开始拖窗口，选中失效 | **P0-2 输入路由** | ❌ |
| 图标按住拖出（OLE 拖文件） | drag_handler → DoDragDrop | 同上，按压被库抢走 | P0-2（Consumed 时不开会话/不抓鼠标） | ❌ |
| 空白区框选（橡皮筋） | WM_LBUTTONDOWN + WM_MOUSEMOVE | 消息全在库手里 | P0-2：Down + Move 转发 | ❌ |
| 双击打开文件 | WM_LBUTTONDBLCLK | 画布根本没处理该消息，直接丢失 | P0-2：转发 DoubleClick | ❌ |
| 右键菜单（图标 shell 菜单 / 框菜单） | WM_RBUTTONDOWN / WM_RBUTTONUP | 同上，丢失 | P0-2：转发右键 | ❌ |
| 滚轮滚动图标区 | WM_MOUSEWHEEL | 同上，丢失 | P0-2：转发 Wheel | ❌ |
| hover 高亮 + tooltip | WM_MOUSEMOVE + SetCursor | move 只驱动库的鼠标光效 | P0-2：转发 move + 宿主可设光标 | ❌ |
| ESC 清除选择等键盘操作 | 宿主窗口可获焦 | 画布 NOACTIVATE，键盘永远进不来 | **P1-1 焦点路由** | ❌ |
| 文件拖入分类框（接收） | IDropTarget 注册在宿主 HWND | 可注册到画布 HWND，宿主按块坐标路由 | 无需改库（用 canvas_handle()） | ⚠️ 需验证 |
| 边缘/四角缩放 + 悬停加粗 border | 自研 HT 边缘 + hover_edge | — | — | ✅ 已有 |
| 拖拽移动 | HTCAPTION 原生移动 | — | — | ✅ 已有（movable） |
| 固定（锁定） | locked + HTCLIENT | — | — | ✅ 已有（set_interactive/set_movable） |
| 框间叠放（激活框最前） | 各自 HWND 的 z 序 | — | — | ✅ 已有（bring_to_front/ZOrder） |
| 移动时对齐参考线吸附 | WM_MOVING 钩坐标 | 块位置由库会话持有，宿主只能事后修正 | P2 move_filter | ⚠️ 可选 |
| DPI / 多屏 | WM_DPICHANGED | — | — | ✅ 已有（MonitorsChanged + 块 scale） |

结论：16 项功能里 8 项因「没有内容挂载点 + 没有事件分发」直接失效，这是 P0 两项为硬阻塞的原因。

---

## 三、API 清单（含建议实现落点）

### P0-1 内容层（每块一张位图，画在玻璃之上）

```rust
impl GlassWindow {
    /// 提交/替换本块内容位图（预乘 BGRA8，物理像素，行距 = w_phys * 4）。
    /// dirty =  Some([x0, y0, x1, y1]) 时只上传/重画该区域（块内物理像素，半开区间）。
    /// 内容层叠放：该块玻璃之上、resize border 反馈之下。
    pub fn update_content(&self, w_phys: u32, h_phys: u32, buffer: &[u8], dirty: Option<[u32; 4]>);

    /// 清除内容层（窗口隐藏/回收时调用，避免 resize 后残留旧内容拉伸）。
    pub fn clear_content(&self);
}
```

建议实现：
- 落点：`runtime/commands.rs` 新增 `UpdateContent/ClearContent` 命令 + `gfx/compositor.rs`
  Pass B 尾部。
- 每块持一张 `ID3D11Texture2D(B8G8R8A8_UNORM, PREMULTIPLIED)` SRV；
  `UpdateSubresource`（或 staging + copy）按 dirty 上传；
  Pass B 画完玻璃块后，对该内容画 textured quad：dst = 块矩形（画布局部坐标），
  UV = dst / 纹理物理尺寸（尺寸不一致时按此规则自然拉伸，宿主负责 ResizeEnded 后重推）。
- 尺寸上限与 clamp 同块尺寸纪律（NaN/负数夹紧，非法尺寸整次调用 no-op）。
- 内容静态场景（分类框常态）零帧间成本：没有 update 就没有上传，只有一次 quad draw。
- 暂不做跨设备共享纹理（宿主是另一个 D2D1 device；CPU 缓冲路径足够，且保留位图语义
  与现有 ULW 管线一致，迁移成本最低）。

### P0-2 输入路由（canvas 消息 → 块 sink）

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButton { Left, Right, Middle }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerPhase {
    /// WM_LBUTTONDOWN（按下，库将依据返回值决定是否开始拖拽会话）
    Down,
    /// WM_LBUTTONUP / WM_RBUTTONUP
    Up,
    /// WM_MOUSEMOVE（含 hover；块内物理坐标持续上报）
    Move,
    /// WM_LBUTTONDBLCLK
    DoubleClick,
    /// WM_RBUTTONDOWN
    RightDown,
    /// WM_MOUSEWHEEL
    Wheel,
}

#[derive(Debug, Clone, Copy)]
pub struct PointerEvent {
    /// 块内物理像素坐标（原点=块矩形左上角，不含 shadow_margin）
    pub block_local: [f32; 2],
    pub phase: PointerPhase,
    pub button: Option<PointerButton>,
    /// Wheel 时为 delta / 120（一格 = 1 或 -1），其余为 0
    pub wheel_delta: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputDecision {
    /// 宿主消费：库不开拖拽会话、不 SetCapture（OLE 拖出因此天然可用）
    Consumed,
    /// 库开始移动会话（等价现状：块内部按压拖动）
    DragMove,
    /// 宿主不处理；库按默认策略走（无 sink 语义时的等价行为）
    Ignored,
}

/// 回调在库渲染线程（canvas wnd_proc 上下文）**同步**执行：
/// 实现方必须微秒级返回、不得阻塞、不得重入引擎（与 emit handler 同一契约）。
pub trait GlassInputSink: Send + Sync {
    fn on_pointer(&self, id: u32, ev: &PointerEvent) -> InputDecision;

    /// 可选：光标覆盖（块内 hover 时）。返回 Some(idc) 时库在 WM_SETCURSOR 中设置；
    /// 默认 None = 库现行逻辑（把手缩放箭头，其余箭头）。
    fn cursor_override(&self, _id: u32) -> Option<isize> { None }
}

impl GlassWindow {
    pub fn set_input_sink(&self, sink: Option<std::sync::Arc<dyn GlassInputSink>>);
}
```

规则建议（关键，直接决定分类框交互正确性）：
1. **resize 把手带优先级恒高于 sink**：命中带内（含块外扩 grip）一律走库缩放会话 +
   hover border 反馈，不询问 sink。
2. `Down`（左键、块内）先询问 sink：返回 `Consumed` → 库本次按压不开会话、不抓鼠标；
   `DragMove` → 进入移动会话；`Ignored` → 按库默认（= DragMove 或 no-op，作者定，建议保持
   现行为即 DragMove，方便 sink-less 场景不变）。
3. `Move / Up / DoubleClick / RightDown / Wheel`：**只要光标落在某块矩形内就转发**给该块
   sink（Up 要转发给按压时消费的同一块）。sink 为 None 的块完全维持现状。
4. 块间取 z 序最高命中（与 hit_test 一致）。
5. sink 的 `Down` 返回值对同一次按压-释放序列生效：Down 后若已 Consumed，后续该序列的
   Up/Move 继续转发该块，期间库不得插手。
6. 落点：`platform/window.rs` 的 canvas wnd_proc（补 DBLCLK/RBUTTON/WHEEL 分支）、
   `core/manager.rs` 提供 hit+sink 查询、`runtime/commands.rs` 存取 sink、`api/handle.rs`
   加 setter。`InputDecision/PointerEvent` 进 `api/` 公共导出。

### P1-1 焦点路由

```rust
impl GlassWindow {
    /// 指定一个宿主窗口句柄（isize 形式的 HWND，通常为宿主的隐藏消息窗口）。
    /// 块被按压且 sink 返回 Consumed 时（或块被点击抬层时），库对该 hwnd 执行 SetFocus，
    /// 使键盘消息（ESC/Delete/F2）可达。画布自身保持 WS_EX_NOACTIVATE 不抢焦点。
    pub fn set_focus_hwnd(&self, hwnd: isize);
}
```

### P1-2（无需开发，只需承诺/验证）

- **文件拖入**：宿主计划把 `IDropTarget` 注册到 `canvas_handle()`（每屏一个），
  用块矩形自行路由与坐标换算。要求库未来样式演进时保持：块区域内画布可被命中测试
  （`WindowFromPoint` 能找到画布）、消息持续被 pump。
- **拖出 OLE**：不依赖库（宿主隐藏壳窗口做 DoDragDrop 锚点），依赖 P0-2 的
  Consumed=不抓鼠标语义即可。

### P2 拖拽位置过滤（对齐吸附，可选）

```rust
impl GlassWindow {
    /// 移动/缩放会话内，库每帧把「候选块矩形（逻辑像素）」交给 filter，用返回值替换。
    /// 用途：宿主的框间对齐吸附（消除“松手才吸回”的跳变）。None = 现行为（仅内置 clamp）。
    pub fn set_move_filter(&self, f: Option<std::sync::Arc<dyn Fn([f32; 4]) -> [f32; 4] + Send + Sync>>);
}
```

---

## 四、宿主（ikun 分类框）侧对应分工（库补齐后由我这边完成）

- 图标命中表 → 线程安全快照（满足 sink 同步回调的微秒级要求）；
- D2D 内容渲染输出 → `update_content` 推流（仅状态变化推 dirty，静态零成本）；
- 每个分类框的交互逻辑从 wnd_proc 消息驱动 → sink 事件驱动（删除 ULW 窗口、命中捕获层、
  透明穿透带、HT 边缘/hover_edge 等全部过渡 hack）；
- IDropTarget 挂各屏 `canvas_handle()`；隐藏壳窗口承担键盘焦点 + OLE 锚点；
- 对齐参考线在会话中按库块矩形轮询绘制（P2 就绪后升级为实时吸附）。

## 五、优先级与工作量速估（库侧）

| 项 | 优先级 | 预估代码量 | 说明 |
|---|---|---|---|
| P0-1 update_content/clear_content | 阻塞 | ~300 行 | commands + compositor Pass B 尾部 + SRV 管理 |
| P0-2 input sink | 阻塞 | ~250 行 | window.rs 补消息分支 + manager 查询 + api 导出 |
| P1-1 set_focus_hwnd | 高 | ~30 行 | Down/抬层路径 SetFocus |
| P1-2 canvas 命中/pump 承诺 | 高 | 0（约定） | 文档注明即可 |
| P2 set_move_filter | 中 | ~50 行 | DragSession.apply 前套一层 |

没有 P0-1 / P0-2，重构无法从「双层叠放」推进到「库窗口即本体」；其余项都有宿主侧降级方案。
