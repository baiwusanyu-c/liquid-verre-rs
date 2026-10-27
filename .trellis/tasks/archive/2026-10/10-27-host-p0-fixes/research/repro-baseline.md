# 修复前复现记录（基线 HEAD=f2dc3d4 / release 0.5.0，2026-10-27 本机实测）

机器环境：2 屏 —— 主屏 3120x2080@192dpi(200%)、副屏 (3120,0)-(5040,1080)@96dpi(100%)。

## 1. 缺陷 #1（重建杀线程）：**完全复现，红**

`tools/verify_monitors_rebuild.py`（完整输出存档 `tools/_repro_rebuild_baseline.txt`）：

```
C1 PASS  枚举到本进程画布 [3280056, 14682482]
C2 PASS  刺激 0.20s 后旧画布 IsWindow=False（重建真的发生）
C3 PASS  出现新画布
C4 FAIL  刺激 8s 后 alive=False + stdout 出现「引擎已退出」
C5a FAIL [metrics] 0 -> 0（线程随 DestroyWindow 而死）
C5b FAIL sink rect 心跳 1 -> 1（块投影停摆）
```

结论：`WM_DESTROY -> PostQuitMessage(0)` 与引擎退出的混用是实锤根因，AC1 脚本判别力成立。

## 2. 缺陷 #3（穿透自锁）：**本地不复现**（prd Open Question 的答案）

`tools/verify_clickthrough_recovery.py` 在**基线（无定时器）**上 6/6 PASS：

```
A1 PASS 停产日志 0.33s
A2 PASS 遮挡移开 0.00s
A3 PASS 光标进块 21ms 摘掉 WS_EX_TRANSPARENT（预算 300ms）
A4 PASS 反向对照 31ms 重新置上
A5 PASS 物理拖拽块移动 160px
```

分析：本地主循环 = `PeekMessageW`（非阻塞）+ DDA 8/32ms 超时 + `!need_render` 8ms sleep
下限，停产期循环照常空转，光标轮询与穿透状态机每圈都跑 => 21ms 即恢复。
宿主的"永久自锁"是其环境特有的停摆路径（宿主实测 900ms+ 不解除、PostMessage 唤醒即恢复
是决定性的）。**修复（WM_TIMER 100ms -> request_render）与宿主止血同构**：不依赖宿主线程
结构，库侧自带兜底驱动源。脚本保留为常驻回归位（若未来本地出现停摆路径，它必须先红）。

## 3. 顺带发现的新缺陷（登记进 design.md，本任务一并修）

### #5 `GLASS_MONITORS` truncate 泄漏孤儿画布（基线实锤）

`GLASS_MONITORS=1` 启动后枚举到 **2 块**画布：runtime/mod.rs 的 boot truncate 只丢
`MonitorRt` 结构，被丢显示器的窗口未 DestroyWindow（USERDATA 未绑定，恒穿透、恒存在，
占 z 序 topmost 带）。复现脚本第一版就因 PostMessage 打到孤儿窗口而"刺激蒸发"。

### #5b `rebuild_monitors` 不应用 monitor_limit（口径不对称）

基线证据：刺激后画布数 1 -> 3（rebuild 重枚举全部屏，truncate 口径丢失）。
修复 = App 构造期缓存 monitor_limit，rebuild enumerate 后同样截断。

## 4. 缺陷 #2 / #4 的基线取证口径

- #2：rebuild 路径不经过 register_class（boot-only），脚本 C7 恒绿属正常；其真复现 =
  同进程两轮 `Engine::new+start+shutdown`（第二轮必报 window-creation）。验证载体 =
  `#[ignore]` 集成测试（真实桌面才能跑，CI 不跑，符合项目取证纪律）+ 单测钉比较逻辑。
- #4：基线 C6 因线程先死退化为"读不到新 rect"。修复后重跑：主屏画布 192dpi => 判别力
  "有"，块 rect 必须逐位不变（旧实现强清 b.scale=1.0 会把宽度重乘 2.0 => 必红）。
  core 单测矩阵（design §4.3）与它互为双保险。

## 5. 复现用的脚本修正记录（教训）

1. 类名枚举必须按 **pid** 过滤（跨进程/孤儿窗口混入 => 打中错窗口把活测成死）。
2. `[metrics]` 是 5 秒墙钟窗口结算，脚本总时长 < 10s 时"数早了"会误报 metrics=0。
3. ctypes 的 FindWindowExW 等原型必须显式声明 restype=c_void_p（64 位截断前科同
   verify_occlusion 注释）。

