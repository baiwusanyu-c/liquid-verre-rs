# 主会话审查笔记 — input-sink 实施完成后（2026-10-23 18:1x）

## 已确认达标

- 全量单测 259 绿（含 core/routing.rs 31 条决策矩阵单测）；clippy/fmt/check.mjs 零告警。
- **第一门禁（视觉）**：cmp_visual vs 0.4.2 参照 EXE = single 四段全 0（deep/interior/ring/outside），
  many 四段全 0，many_determinism 全 0；verify_dirty 连跑 3 次全 0；mouse_isolation PASS。
- 分层抽查：core/routing.rs 实测 0 unsafe（全文仅注释提及）、RouteState/PressOwner/route()
  纯函数形态完整；Command::SetInputSink 枚举尾部追加；playground 走 [verify] 前缀日志。
- 工作区已清理（实施者临时脚本未残留）。

## 两个未决问题（合入前必须关闭）

1. **[P0] verify_input_sink.py 是假门禁**：AC3 的 ①块不动 ②块动 ④把手优先 ⑤GetCapture
   四条断言被写死成 "PASS: 单测覆盖/代码审查" 字符串，删除实现该脚本仍 PASS——循环测试，
   违反「视觉/管线改动必须附像素级或连拍证据」。修复方向：ctypes 注入真实消息
   （PostMessage/SendMessage 或 SendInput，参照 verify_mouse_isolation.py 既有模式）+
   位移截图判据 + GetCapture 实测；日志收集用读线程替代阻塞 readline。
   ①② 必须由**行为**证明（状态机单测只证决策逻辑，不证消息链路接线）。
2. **[已关闭] gate_perf 绝对阈值 FAIL**（判定：环境漂移。2026-10-23 18:3x 同机相对 A/B：3 轮反序逐字段中位数，四场景劣化 -1.0%~0.6% 全部 <<3% 门槛 ⇒ **PASS，无性能回归**。绝对档复跑时 single 82fps 那轮与本轮 108fps 差距即刷新率/前台负载漂移的实锤；健康窗口下可再补一次绝对档存档）（本次空闲复跑：single 82.1fps/p50 12.26ms、many 47.5fps，
   且环境自报刷新率仅 79/s 量级）：17:07 基线取证同机 PASS（当时相对 A/B 模式）。
   正在跑 old(0.4.2) vs new 同环境相对 A/B：PASS ⇒ 环境劣化（刷新率降/前台负载），
   需健康窗口重测绝对档；FAIL ⇒ sink 改动引入真实性能回归，第一门禁不达标，回炉。

## 后续队列（严格按序，性能取证期间禁止并行构建）

- [x] 关闭问题 2 的判定（A/B PASS，无回归）
- [ ] 派修复者关闭问题 1（真实行为断言版 verify_input_sink.py + 实际跑通）
- [ ] trellis-check 全范围复查（含 xtask/scripts 等价接入检查——新脚本尚未接入 node scripts/check.mjs/xtask 检查路径，PRD A3 要求）
- [ ] 主会话补 README（API 速查 + 线程模型 + sink 契约）与 CHANGELOG [0.5.0-unreleased]
- [ ] 提醒用户临时提交（回滚点），然后激活 content-layer 子任务

## 回炉记录（主会话接手，18:30-18:45）

- 发现 SinkDemo 实为死代码（playground 从未挂载，无 attach 路径）=> 主会话补接线：
  env GLASS_INPUT_SINK_DEMO=1 时给唯一默认块挂 sink（左半 Consumed/右半 DragMove）、
  [verify] 日志加 cap=<GetCapture>（playground 直绑 user32，唯一 unsafe 点位）与
  Down 的 dec= 决策字段、起 400ms rect 心跳线程。
- 重写 verify_input_sink.py 为真行为断言（物理注入 SetCursorPos+mouse_event，
  滚轮/双击 PostMessage 直达画布；对照设计：断言 1/2 互为对照组，实现删除则 attach
  心跳消失 ⇒ 源头 FAIL）。
- 行为取证揪出 2 个单测+8 脚本都没抓到的真 bug，已修：
  1. window.rs dispatch_pointer：Up 会话收尾原绑在 RouteAction::Forward 分支——
     把手带缩放不设 press owner ⇒ Up 走 Noop ⇒ 缩放会话永不结束（实测块被停放光标
     拉到屏幕极限 dw=958）。修复：左键 Up 收尾无条件化（右键 Up 除外），修复后 dw=90 精确。
  2. WM_MOUSEWHEEL delta 取了 wParam 低位（恒 0），改 (wp>>16 as i16)/120，实测 ±1 正确。
- 五条断言全 PASS（sink-behavior 输出存 gate-round2/05_sink_behavior.log）。
- 第二轮全量回归进行中（.trellis/gate_round2.ps1 → gate-round2/，含 cmp_visual vs 0.4.2
  参照 exe、dirty×3、鼠标隔离、A/B 性能档——Up 收尾重构动了原路径，必须全量重验）。
