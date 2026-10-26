# Design — P0-2 输入路由

架构与决策顺序见父 design.md §2；本文件是落点细节。

- 决策状态机（0-unsafe，进 `core/`）：新增 `core/routing.rs`（或并入 `manager.rs`，倾向新文件）
  纯函数 `route(state, hit_resize, hit_test, event) -> (Action, state')`，
  `Action = StartMove(block) | StartResize(block,edges) | Forward{sink_block, event} | Consume | Noop`。
  `PressOwner{block}` 是其状态位。全部转移表单测（AC2 矩阵）。
- `platform/window.rs`：只负责 Win32 拆包（lp 坐标→屏幕→块内物理像素换算，复用现有 MOUSEMOVE
  路径的换算与 `read_cursor()` 纪律——窗口过程与主循环必须同一光标源，AGENTS.md 坑 3），
  调用状态机，执行 Action。新增四消息分支都走同一条 `dispatch_pointer` 汇聚函数，避免分支复制。
- `input/drag.rs`：移动会话开启时机后移——由 window.rs 在状态机判出 `StartMove` 后才建会话/
  SetCapture；`Consumed` 路径完全不进 drag.rs（不抓鼠标是 FR4 的实现保证）。
- sink 存储：`Command::SetInputSink { id: u32, sink: Option<Arc<dyn GlassInputSink>> }`；
  runtime 层每块 `Arc` 槽位（读写分离：setter 走命令帧首换，消息读路径直接克隆指针引用，
  零分配；热路径 Move 转发 = 一次虚调用 + 平凡 struct 填充，禁格式化日志）。
- `api/`：新模块 `api/input.rs`（trait + 事件 + 决策类型 + 契约文档注释），`api/mod.rs` re-export；
  `GlassWindow::set_input_sink` 加在 `api/handle.rs`。trait 方法签名与清单 §P0-2 一致
  （`on_pointer(&self, id:u32, ev:&PointerEvent) -> InputDecision`）。
- DoubleClick 语义：Win32 的 DBLCLK 本身携带「第二个 Down」，本项目**只转发 DoubleClick 一条**，
  不补发 Down（状态机上把 DBLCLK 当作独立事件进 owner 判定：若已有按压 owner 则转发 owner；
  否则按光标所在块转发，返回值仅作记录不产生会话——避免双击触发拖动）。
- 右键序列：RightDown 询问返回值？——**不询问**（右键不参与拖拽会话仲裁，返回值无意义但类型统一，
  契约注明宿主可返回 Ignored）；只保证转发成对到达。
- WM_SETCURSOR：现有分支后插入 cursor_override 查询；sink=None 零成本路径不变。
- 危险点（rustdoc 反例）：①回调内 DoDragDrop（卡渲染线程）；②回调内调 poll_events/同步读命令
  结果（重入）；③回调内长循环/加自己的锁等待 UI 线程（宿主死锁）。

## 取证设计

- `verify_input_sink.py` 通过 playground demo 的 sink 日志（`[verify]` target）+ 截图位移判定：
  块「动了」= 稳态帧玻璃位置变化（复用 cmp 工具的块矩形读取）；不引入新日志前缀接口之外的通道。
