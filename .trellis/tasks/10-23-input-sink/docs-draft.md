# docs-draft — input-sink 的 README/CHANGELOG 素材（主会话起草，集成时落正式文件）

## 待修约定违规

- crates/liquid-verre/src/api/handle.rs set_input_sink 的文档注释是英文（"Set input routing sink."），
  项目约定中文。trellis-check 时顺手修。

## README「API 速查」追加段（草案）

### 输入路由（GlassInputSink）

让宿主把「块」当窗口用：块内指针事件会同步路由到你实现的 sink，你可以决定这次按压
是否让库开移动会话。

```rust
use liquid_verre::api::{GlassInputSink, InputDecision, PointerEvent, PointerPhase};
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

window.set_input_sink(Some(Arc::new(MySink)));
```

规则（硬性的）：

1. **缩放把手带优先级恒高于 sink**：命中把手带（含块外扩 grip）一律走库缩放会话，不询问 sink。
2. 块内 `Down` 询问 sink：`Consumed` = 库本次按压不开移动会话、不 SetCapture（OLE 拖出天然可用）；
   `DragMove` = 库开移动会话；`Ignored` = 库默认（等价 DragMove）。
3. Down 之后的同一按压序列，Move/Up 粘滞转发给同一块（Up 必回按压时的块，即使光标已离开）。
4. 未按压时 Move/DoubleClick/RightDown/RightUp/Wheel 转发光标所在最上层块的 sink。
5. 未设置 sink 的块行为与旧版本完全一致（零迁移成本）。
6. 回调在**渲染线程**同步执行：微秒级返回、不阻塞、不重入引擎、**禁止 DoDragDrop**
   （OLE 模态泵会卡死渲染线程；拖出请把 DoDragDrop 锚在宿主自己的壳窗口上）。
   回调内调库 setter 是安全的（异步入队，≤1 帧生效）。

## CHANGELOG [0.5.0-unreleased] Added 段（草案）

- 新增 `GlassInputSink` 输入路由：块内指针事件（Down/Up/Move/DoubleClick/RightDown/Wheel）
  同步路由给宿主，Down 返回值决定库是否开移动会话（Consumed=不开会话不抓鼠标，支持 OLE 拖出）；
  缩放把手优先级恒高于 sink；cursor_override 支持宿主覆盖块内光标。未设置 sink 的块行为不变。
- 新取证脚本 `tools/verify_input_sink.py`（五条行为断言：Consumed 块不动 / DragMove 块动 /
  事件到达 / 把手优先 / GetCapture 归属）。

## 门禁台账（本子任务）

- 单测 259 全绿（含 routing 31）；clippy/fmt/check.mjs 零告警。
- cmp_visual vs 0.4.2：single/many/determinism 全 0；verify_dirty x3 全 0；mouse_isolation PASS。
- gate_perf 相对 A/B：四场景 -1.0%~0.6%，PASS 无回归（gate_ab_run.log）。
- 待补：verify_input_sink.py 真行为版结果；绝对档在健康窗口的一次存档。
