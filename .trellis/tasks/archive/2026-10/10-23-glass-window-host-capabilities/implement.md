# Implement — 父任务执行计划（任务树级）

各子任务的步骤清单在自己的 implement.md；本文件只管顺序、门禁与回滚。

## 顺序与门禁

- [ ] 0. **基线留证**：在 HEAD（0.4.2）跑一遍全套门禁作为参照（取证图 + 数字存本目录
  `baseline-evidence/`）。此后所有「零差异」比对都以此为参照。
- [ ] 1. 子任务 `input-sink`（交互骨架先行；P1-1 依赖它的 Down/Consumed 路径）。
- [ ] 2. 子任务 `content-layer`（纯渲染增量；若与 1 并行，共改面 =
  `runtime/commands.rs`/`api/handle.rs`，命令变体一律追加在枚举尾部，谨防互踩）。
- [ ] 3. 子任务 `focus-drop-routing`（依赖 1 完成）。
- [ ] 4. **集成评审（父任务职责）**：
  - 跨子任务数据流走查：Down→Consumed→SetFocus→Up 粘滞回同一块；内容绘制层序
    与 sink 命中 z 序一致；border 反馈仍在内容之上。
  - 全量门禁 = 父 prd.md §4 全套（见下表）+ A1–A5 逐项核对。
  - playground demo 与 README/A4 宿主端到端样例文档齐备。
- [ ] 5.（可选）子任务 `move-filter`。

## 全量校验命令（集成评审时全部执行）

```powershell
cargo test --release --workspace
cargo clippy --workspace --all-targets --release -- -D warnings
cargo fmt --all -- --check
node scripts/check.mjs
python tools/gate_perf.py
# 8 回归脚本：verify_dirty(连跑>=3) / verify_mouse_isolation / verify_validity /
# repro_tremble far / repro_tremble many / verify_adaptive / ab_latency / ab_memory
```

取证窗口纪律：性能取证期间禁止并发 cargo build/test（劣化窗口）；机器健康判据
`rel_max < 5 ms`、`acq_max < 25 ms`；轮转 A/B ≥3 重复取中位数。
视觉取证只看 `GLASS_INTERNAL_SHOT` composed 图；取证脚本显式设 `GLASS_DEFAULT_BLOCK=1`。

## 提交与回滚

- 不自动 commit（用户规则 C7）。每个子任务门禁全绿到达回滚点位时，**提醒用户**发起一次
  临时提交；用户不同意则以 patch 快照代替回滚点。
- 回滚点 = 各子任务验收快照；总回滚 = `a23cd99`（0.4.2 release commit）。

## 评审门（Phase 1.4 之前必须过）

- 用户评审：父 prd/design/implement + 各子任务工件。
- 通过后按顺序 `task.py start` 对应子任务进入 Phase 2（inline 或 dispatch 由当时会话决定；
  dispatch 模式需先做 Step 1.3 配 implement.jsonl/check.jsonl）。

## 执行台账（主会话维护，2026-10-26）

- [x] 0. 基线留证：16 步全绿（baseline-evidence/，参照 exe_0.4.2-playground.exe）。
- [x] 1. 子任务 input-sink：实施+行为取证+两轮门禁+trellis-check 7/7 全过；
      附带抓到并修复 2 个存量级 bug（缩放会话收尾/wheel 高位字）。
      门禁数值：cmp_visual single 四段全 0；gate A/B -1.1%~0.3%；verify_dirty×3 全 0。
- [ ] 2. 子任务 content-layer：Erdos 首轮交付**验收不通过**（批量 pass 叠放不等价 /
      verify_content.py 弱证 / AC2 单测缺），已下发返工清单（真交错 + 硬 FAIL 断言 +
      判别力自检 + core 校验单测），返工中。
- [x] 3. 子任务 focus-drop-routing：实施闭环（15:52 全绿）。过程重大裁决两条：
      ① 主会话推翻实施者「FR3 100% 漏检需降级」结论（对照实验证明机制健康；根因是测量方法：
      块外误计/穿越式采样/孤儿实例污染，v6 重写后 450/450 + 反向对照）；
      ② 主会话实锤 AttachThreadInput 代调 SetFocus 会挂死渲染线程（插桩二分定位：
      consume_focus_enter 后心跳停摆、进程活着线程冻结），库侧焦点 FFI 全部删除，
      语义改造为 EngineEvent::FocusHint + 宿主同线程自聚焦（README/CHANGELOG/rustdoc 三处
      同步改写）。verify_focus_drop v6 含防死锁心跳断言与连击 100（按压事件=Down+DBLCLK，
      兼容系统双击合成两态）。round-3 全量 17 步全绿（gate-round3/，含 cmp 四段 0、
      A/B 档、连击、内容层/输入路由回归）。待：trellis-check 复查。
- [ ] 4. 集成评审。
- [ ] 5.（可选）move-filter。

## 已决议题（2026-10-26 拍板）

用户提出架构路线再审：**「与其在画布上重造窗口能力，不如直接在真 Win32 窗口上做玻璃」**。
研究结论文档：本目录 `research/architecture-native-window.md`（trellis-research 在办）。
决策结论：**用户拍板维持现路线（方案 C：库只做玻璃，窗口本体归宿主）**，不启动窗口化重写；per-window 若未来要做，只允许作为主线外的实验 spike。focus/move-filter 按原计划推进；
content-layer 返工按既定路线继续（沉没成本小，且其行为取证/门禁资产两条路线通用）。
