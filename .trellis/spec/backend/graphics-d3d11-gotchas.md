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
