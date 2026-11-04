# Graphics / D3D11 / Windows 静默陷阱清单

每条都曾在本项目真实踩中并取证。**新增 GPU/窗口代码前逐条对照。**

1. **`ClearRenderTargetView` 忽略 scissor。** 想"只清脏区"必须用带 scissor 的零填充绘制，
   或接受整屏清。本项目选择整屏清 + 画全部块（见 rendering-pipeline 不变量 1）。
2. **flip-model back buffer 在 Present 后内容不保。** `CreateSwapChainForComposition` +
   `FLIP_SEQUENTIAL` + BufferCount=2 下 `GetBuffer(0)` 指针可能不变但内容未定义。
   ⇒ 帧与帧之间不得假设画布持久；"增量重画"只适用于 Pass A 这类**自有 RT**。
3. **hazard 静默丢绘制。** 目标纹理的 RTV/SRV 还绑在管线上时 `CopyResource`/绘制会被运行时
   丢弃且**无错误码**。拷贝/换 RT 前必须 `OMSetRenderTargets(None)` + `PSSetShaderResources(null×9)`。
4. **windows 0.39 的 `IDXGIOutput5::DuplicateOutput1` 绑定错误**（把 ColorSpace 槽位当成
   格式数组）。不要用；HDR 需求走 WGC 自动切换。
5. **HDR 桌面的 DDA 复制面是 HDR 参考值**：FP16(scRGB 线性) 与 8bit(G22-P2020) 两种容器都见过，
   直接当 sRGB 用 = 整张过曝（实测 R/G/B≥255 比例 52/70/84%）。
6. **`Instant - Duration` 在开机时间小于被减数时 panic**（`Instant` 自 boot 起算）。
   一律 `checked_sub(..).unwrap_or_else(Instant::now)`。
7. **debug 构建整数溢出即 panic。** 一切来自外部（DDA 脏/移动矩形、面板输入）的整数运算先
   sanitize/clamp 或 widen 到 i64。`sanitize_rect` 有单测盯着。
8. **GPU timestamp query 会打断驱动批处理**（实测帧率打到 15fps）。禁止进闭环判据；
   只在 `GLASS_GPU_TIMINGS=1` 的离线归因里用。
9. **`Copy-Item` 保留源 mtime** → cargo 会误判"无需重建"，测到旧二进制。恢复源码后必须
   `touch`/重编并核对产物时间戳。
10. **WDA_EXCLUDEFROMCAPTURE 使系统截屏看不到玻璃** ⇒ 视觉取证只能走 `GLASS_INTERNAL_SHOT`
    的 composed 图；不要用 OS 截屏判断视觉效果。
11. **Private Bytes 在本机同进程内可漂 ±50MB、双屏有 +52MB 双峰** ⇒ 内存结论必须轮转交错
    A/B + 稳态窗口中位数（`tools/ab_memory.py`），单次 max/median 都不可信。
12. **scissor 局部重算模糊链不幂等** ⇒ 下采样外扩 14/12/8/0、tent 链 32/24/16/8、首级外接框⊕48；
    改任何一级 kernel 必须重算这组数并跑 `verify_dirty` 多次。
13. **`Error::code()` 是 HRESULT，不是 Win32 错误码。** `GetLastError()=1410
    （ERROR_CLASS_ALREADY_EXISTS）` 进 `HRESULT_FROM_WIN32` 变成 `0x80070582`；
    与裸 `1410` 做数值比较**恒不等**（0.5.0 前科：register_class 的"类已存在"容忍分支
    注释写了、代码里永不命中，同进程重建第二个 Engine 必报 window-creation）。
    要容忍就比 `HRESULT::from_win32(1410)` 全值；掩码低 16 位会与其他 facility 撞车。
14. **`WM_DESTROY` 里无条件 `PostQuitMessage` = 把「资源销毁」混用成「引擎退出」。**
    显示器变化重建画布对旧窗口 `DestroyWindow` 会顺手杀死渲染线程消息循环
    （宿主实测：热插拔后玻璃永久消失）。退出意图必须显式（本项目 = `App::shutdown`
    判定 + 销毁前清 `GWLP_USERDATA` 双保险）；取证 `tools/verify_monitors_rebuild.py`。
15. **`Vec::truncate` 丢的是结构，不是窗口。** boot 期 `GLASS_MONITORS` 截断只 drop
    `MonitorRt`（gfx 资源随 drop 释放），**HWND 本体留在桌面上**成孤儿（恒穿透、占
    topmost z 带、吞发给它的消息——复现脚本第一版就被它把刺激吃掉）。丢窗口元素必须
    显式 `DestroyWindow`，顺序 = 先 drop 绑定该 HWND 的 swapchain/采集、再毁窗。
16. **叠加 pass 的层序是「盖在全部之上」，不是「盖在自己那块之上」。** 缩放反馈 border 曾
    在 `draw_blocks` 之后统一发射：逐块 scissor 只对**块间**生效，对**玻璃层**永远是
    「后画 = 压在上面」⇒ 两块重叠时拖下层框的边缘，白框画到上层框身上（宿主实测，
    `raise_box` 只修正在拖的那块，修不了 hover 阶段）。正确结构 = **逐块内联**
    （画完第 i 块玻璃及其内容，立刻画第 i 块 border），判据本体是纯函数
    `core::draw::border_emit_plan`。配套纪律两条：① `draw_blocks` **每块开头无条件重设
    全套玻璃管线状态**，子 pass 一律不写「还原」代码；② 内联版**绝对不许**在尾部
    `OMSetRenderTargets(None, None)` —— 之后还要往同一张 RT 画更高块的玻璃，解绑 =
    下一块整块闪黑（0.5.2 #1）。取证 `tools/verify_border_zorder.py`。
17. **「还原清单」漏一项就整块消失，而 cbuffer/LUT 账本比管线更容易漏。** 0.5.2 同处发现
    两处：内容 pass 结束后整体重绑 `t0..t8` 把 `t4` 打回默认 LUT，但 `bound_lut` 仍记着
    自定义键 ⇒ 下一块同键跳过重绑、采到**错的折射 LUT**（无错误码、无告警）；旧
    `draw_borders` 尾部 `OMSetRenderTargets(None, None)` 则会让内联后的下一块玻璃画不出来。
    规矩：**绑定状态与「记账的键」必须同处更新**；能整体重设就不要做差异还原。
18. **`ShowWindow` 不是「只改可见性」，它同时是一次 Z 序操作。** `SW_SHOWNOACTIVATE` 把窗口
    抬进**普通带顶部**。「创建时置底 → 首帧就绪才显示」的时序因此必然在显示那一刻
    把画布弹到最前（宿主 EnumWindows 4ms 采样实测 t+0.6s）。显示与层级必须同一次
    `SetWindowPos(..., HWND_BOTTOM, SWP_SHOWWINDOW)` 完成（0.5.2 #2）；同族结论见
    `platform/level.rs`：**不能**按「扩展样式变没变」决定是否发 `SetWindowPos`。
19. **窗口类少了 `CS_DBLCLKS`，双击消息就根本不存在。** 不是「库没处理 `WM_LBUTTONDBLCLK`」，
    而是 Windows 只对带该样式的类发送它 ⇒ 库里那条 `PointerPhase::DoubleClick` 分支是
    **永不执行的死路径**，单测与代码审查都看不出来（0.5.2 #5，宿主日志实锤）。类样式
    现集中成 `CANVAS_CLASS_STYLE` 常量 + 类级单测；物理取证 `tools/verify_double_click.py`
    还要反向排除「库自己合成双击」。
20. **`u8` 连击计数器 + 裸 `+1` = 定时炸弹。** `quality.rs` 的 `good_streak/bad_streak`
    在 debug 构建约 256 个同向窗口（≈256 s）后 `attempt to add with overflow` panic
    **杀渲染线程**，release 构建则回绕成 0 静默毁掉升档判据（0.5.2 #6，用户读作
    「玻璃全灭」）。与第 7 条同源：**任何累计量都要 saturating + 夹紧**，并把判据抽成
    纯函数、单测「顶到上限再跑一轮」。
