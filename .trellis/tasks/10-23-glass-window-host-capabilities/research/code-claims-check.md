# Research — 清单论断 vs 0.4.2 代码现状核对（2026-10-23）

| 清单论断 | 核对 | 证据 |
|---|---|---|
| 画布仅处理 5 条输入消息（NCHITTEST/LBUTTONDOWN/UP/MOUSEMOVE/SETCURSOR） | ✅ | `platform/window.rs`:176-244（另有 DPICHANGED/DISPLAYCHANGE/DESTROY/KEYDOWN-ESC，均非块内输入路由） |
| 右键/双击/滚轮消息落 DefWindowProc，宿主收不到 | ✅ | 同上，无对应分支 |
| 画布 `WS_EX_NOACTIVATE`，键盘进不来 | ✅ | `window.rs`:101；:236-244 注释自认 ESC 路径正常拿不到焦点 |
| Pass B 只画玻璃，宿主像素无挂载点 | ✅ | `gfx/compositor.rs` |
| 建议落点文件全部存在 | ✅ | `runtime/commands.rs`、`gfx/compositor.rs`、`core/manager.rs`、`api/handle.rs`、`platform/window.rs` |
| `GlassWindow` 为真实公共句柄类型 | ✅ | `api/handle.rs`:365 |
| 同步回调契约有先例 | ✅ | `api/event.rs` `EventHandler`：「渲染线程同步回调；不阻塞、不重入（会死锁）」 |
| 块命中已有单一出口 | ✅ | `core/manager.rs`:81 `hit_test`（z 序最高）、:96 `hit_resize`——决策状态机可做成 0-unsafe 纯函数 |
| 拖拽会话状态位置 | 清单未提 | `input/drag.rs`（+ `runtime/app.rs` 驱动）——Down 重排的改造点在这里，不是 manager |

## 清单没写透、规划已纳入的风险

1. `WS_EX_TRANSPARENT` **逐帧切换**（`window.rs`:432-443）⇒ OLE 拖入 `WindowFromPoint` 存在
   一帧竞态；P1-2 不能只是「文档承诺」，要实测（降级预案见父 design §3）。
2. sink 回调内起 `DoDragDrop` = OLE 模态泵卡死渲染线程（玻璃停绘）。清单§四已把锚点放宿主
   壳窗口，规避成立，但**必须写进 API 契约文档**，否则第一个集成方就会踩。
3. 内容层 dirty 与失效/影响盒机制（不变量 5）必须统一收口，否则 `verify_dirty` 阀门会被打断。
4. cbuffer c19 起新增（c0..c18 冻结）；内容 PS 独立小 shader，仿 border pass 模式。
5. `window.rs` 的 ESC→Shutdown 是现存死路径；P1-1 给焦点体系开后需重新评估（宿主误把焦点
   给画布时可能意外关停引擎）。
6. PointerButton 需要 Right ⇒ Up 也有右键形态，右键菜单才能 Down/Up 成对；清单事件枚举里
   只有 RightDown，缺 RightUp——已在 P0-2 PRD 补上。中键本轮不做。
7. 两个 P0 子任务共改 `runtime/commands.rs` 与 `api/handle.rs`，并行实施需约定追加位置（枚举尾部）。

## 工作量口径（来自清单，已按代码现状校准）

P0-1 ~300 行、P0-2 ~250 行、P1-1 ~30 行、P1-2 文档+验证、P2 ~50 行；
另加：取证脚本 2 个（content/input 各一）、playground demo、README/CHANGELOG/rustdoc。
