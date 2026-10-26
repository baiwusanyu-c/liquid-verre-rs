# 集成评审清单（父任务 Step 4 预置，等 P1 落地后逐项打钩）

## A. 跨子任务数据流走查（读代码验证，不跑工具）

- [ ] Down → Consumed → SetFocus(宿主) → Up 粘滞回同一块：dispatch_pointer Consume 分支到
      runtime focus 存储再到左键 Up 收尾，一条链读通，无中途丢状态。
- [ ] 内容绘制顺序（逐块交错 z 序）与 sink 命中 z 序（manager.hit_test）用同一 z 序源。
- [ ] border 反馈 pass 在全部内容之上（最后叠加）——三层序终态：玻璃→内容→border。
- [ ] 守卫链：set_focus_hwnd(画布自身) 拒绝；sink 回调内 setter 合法（异步入队）；
      ESC→Shutdown 仍是死路径（守卫保证画布不经库拿焦点）。
- [ ] 命令枚举尾部顺序 = SetInputSink → UpdateContent → ClearContent → SetFocusHwnd，
      无插队改序（历史取证脚本对既有序号无依赖，但保持纪律）。

## B. 全量门禁（一次跑齐，出数字进 baseline-evidence/integration/）

- [ ] cargo test --release --workspace（预期 ≥246 全绿）
- [ ] clippy -D warnings / fmt --check / node scripts/check.mjs
- [ ] cmp_visual：新构建 vs baseline exe，single 四段全 0（sink-less + content-off 配置）
- [ ] verify_dirty ×3、verify_mouse_isolation、verify_validity、repro_tremble far/many、
      verify_adaptive、ab_latency、ab_memory
- [ ] verify_input_sink、verify_content、verify_focus_drop（三条新行为取证全绿）
- [ ] gate_perf 同机 A/B（3 轮反序）——健康窗口判据达标
- [ ] 内容块在场时的抽测一轮（checker 场景下 many-drag fps 不越 gate）

## C. 遗留决策项（评审时拍板）

- [ ] verify_validity 不覆盖内容场景（Newton 遗留风险）：本轮补 or 记入 0.5.1 backlog。
- [ ] verify_content.py 的 [2/5] 旧弱断言是否仍在（Erdos 返工后主会话只抽查过前半段——
      验收时逐条核 ①~⑤ 与 OVERLAP_DATA 判别力注释）。
- [ ] vc_interleaved.exe / probe 产物清理：保留 vc_batch_probe.exe（回归判别力复用），
      其余移 research 存档或删除。
- [ ] task.json base_branch=master 告警（archive 会拒绝 PR 目标）：本项目走本地 master
      人工提交流，归档前用 task.py set-meta/文档说明绕开，不造分支。
- [ ] README「拖入承诺」小节数字与修正后 verify_focus_drop 实测一致（Russell 修完核对）。

## D. 交付面（宿主对接）

- [ ] ikun 需要的 API 全部有公共导出 + rustdoc 线程契约（含 DoDragDrop 禁令）。
- [ ] README：输入路由 / 内容层 / IDropTarget 三节互链完整；最小宿主集成样例（A4）在
      README 或 docs/ 成文。
- [ ] CHANGELOG [0.5.0-unreleased] 覆盖三能力 + 三取证脚本。
- [ ] 提醒用户：临时提交（本清单 B 全绿之后是唯一安全提交点）；发布走 release.mjs 流程
      由用户另行发起。
