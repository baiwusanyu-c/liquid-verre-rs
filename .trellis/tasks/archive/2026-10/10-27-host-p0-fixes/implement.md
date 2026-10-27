# Implement: 宿主实测四缺陷修复

顺序原则：先小后大、先易证后难证；每一步结束都有可运行的验证命令。
所有命令在仓库根执行；被测 EXE = `target/release/liquid-verre-playground.exe`。

## Step 0 — 修复前复现基线（先证病，再开药）

- [ ] `cargo build --release -p liquid-verre-playground`（基线 HEAD 未改动状态）
- [ ] 写最小版 `tools/verify_monitors_rebuild.py`（能跑即可，断言集按 design §5）并运行
  → 预期 FAIL（引擎死于重建）；把输出存 `research/repro-baseline.md`
- [ ] 写最小版 `tools/verify_clickthrough_recovery.py` 并运行（基线无定时器）
  → 记录本地能否复现穿透不解除（prd Open Question 的实测答案）
- [ ] 复现记录落盘后开始改码

## Step 1 — #2 register_class（独立最小修复）

- [ ] `platform/window.rs`：新增 `is_class_already_exists` 纯函数（含注释：
  HRESULT 低 16 位 / `0x80070582` 前科），register_class 错误分支改用它；
  优先 `windows::core::HRESULT::from_win32(1410)`（确认 0.58 可用则用构造值）
- [ ] `#[cfg(test)]` 单测：0x80070582→true；0x80070005/裸 1410/0→false
- [ ] 验证：`cargo test --release -p liquid-verre-rs window`（或全量单测）

## Step 2 — #1 WM_DESTROY 退出意图显式化

- [ ] `platform/window.rs` WM_DESTROY：`app_of(hwnd).is_some_and(|app| app.shutdown)`
  才 PostQuitMessage；注释写明「画布销毁 ≠ 引擎退出」+ 双保险（unbind 先行）
- [ ] 验证：`cargo build --release` 通过；Step 5 的 verify_monitors_rebuild 转绿（在 Step 4
  完成后统一跑）；另手工确认 `shutdown()` 后进程退出（AC1 终局断言）

## Step 3 — #3 画布 100ms 兜底定时器

- [ ] `platform/window.rs`：`TIMER_ID_WAKEUP` 常量（注释登记唯一性）；
  `create_canvas` 成功后 `SetTimer(hwnd, TIMER_ID_WAKEUP, 100, None)`；
  wnd_proc `WM_TIMER` 按 wp 过滤 id → `request_render(hwnd)` → LRESULT(0)
- [ ] 注释写「为什么」：停产期无帧时钟，穿透状态机/遮挡复检不得依赖鼠标消息自身唤醒；
  引用宿主实测数字（900ms+ 不解除、PostMessage 唤醒即恢复）
- [ ] 验证：`verify_clickthrough_recovery.py` PASS；静止段 `[metrics]` 无 CPU 阶跃
  （gate_perf 在 Step 6 统一跑，此处先肉眼过一轮 fps/rel）

## Step 4 — #4 rebuild 重估（纯函数抽取 + 两处行为修复）

- [ ] `core/geometry.rs`：`index_at`（自 platform/display.rs 下沉，纯算术）；
  `platform/display.rs::index_at_rects` 改薄转发（单一来源）
- [ ] `core/block.rs`：`clamp_geom`（自 Block::clamp 抽算术，Block::clamp 复用）；
  `rehome_block` + `MonitorGeom`/`BlockGeom`（scale 下限 0.05、除零 1e-3 口径
  与 runtime 现状逐位一致）
- [ ] `runtime/app.rs::sync_block_monitor` 变薄壳：构造几何 → rehome_block → 写回；
  命令路径行为不变（用现有 verify_crossscreen / tremble 回归兜底）
- [ ] `runtime/app.rs::rebuild_monitors`：`app.monitors = next` 后立即
  `app.refresh_monitor_rects()`；删除 `b.monitor = 0; b.scale = 1.0;` 强清；
  注释记录 ①②两个真根因与 2x 膨胀前科
- [ ] 单测矩阵 6 条（design §4.3）全部落进 `core/block.rs` tests
- [ ] 验证：`cargo test --release -p liquid-verre-rs`；`verify_monitors_rebuild.py` PASS

## Step 5 — 取证脚本定稿（判别力自检：删实现还红吗）

- [ ] `verify_monitors_rebuild.py`：断言集 = AC1 全部 + AC4 行为面 + #2 重建路径回归
  + shutdown 终局；反向对照 = 无重建日志/引擎死亡即 FAIL
- [ ] `verify_clickthrough_recovery.py`：300ms 解除断言 + 块外穿透仍在反向对照 +
  物理拖动块终局断言；无停产日志 = FAIL（场景没搭起来，禁止空转 PASS）
- [ ] 两脚本各连跑 ≥3 次（间歇性问题纪律，参照 verify_dirty 前科）

## Step 6 — 全量质量闸门（提交标准，但本任务不 commit）

- [ ] `cargo test --release --workspace`（154 基线 + 新增单测）
- [ ] `cargo clippy --workspace --all-targets --release -- -D warnings`
- [ ] `cargo fmt --all -- --check`
- [ ] `node scripts/check.mjs`
- [ ] `python tools/gate_perf.py`（性能取证期间禁止并发构建 —— 项目红线）
- [ ] BASELINE 回归复跑：`verify_dirty` ×3、`verify_mouse_isolation`、`verify_validity`、
  `repro_tremble far`、`repro_tremble many`、`verify_adaptive`、`ab_latency`、`ab_memory`
- [ ] 0.5.0 功能取证复跑（触碰 wnd_proc/输入/焦点路径）：`verify_input_sink`、
  `verify_content`、`verify_focus_drop`、`verify_occlusion`、`verify_crossscreen`

## Step 7 — 文档与规格回写

- [ ] `CHANGELOG.md` 新增 `## [0.5.1-unreleased]` Fixed 四条（每条带现象一句话）
- [ ] `.trellis/spec/backend/graphics-d3d11-gotchas.md`：追加两条静默陷阱——
  「HRESULT ≠ Win32 错误码（低 16 位掩码/相等比较陷阱）」、
  「WM_DESTROY 无条件 PostQuitMessage = 把资源销毁混用成退出信号」
- [ ] `README.md` 若有「线程模型/生命周期」段落涉及重建语义，同步一句「显示器变化
  重建画布不会结束引擎」
- [ ] `implement.jsonl` / `check.jsonl` 若被后续子代理复用，登记本任务关键 spec 条目

## Review Gates

- G1（Step 4 后）：新增单测全绿 + sync_block_monitor 既有行为回归脚本先跑一遍
- G2（Step 5 后）：两条新脚本 3 连 PASS + 基线复现记录对比成立
- G3（Step 6）：五连全绿 + 回归清单全绿，才允许汇报完成

## Rollback Points

- 基线 = master HEAD `f2dc3d4`（release: 0.5.0）。
- 步骤间回滚：Step 1-4 各自独立可 revert（互不依赖）；若 #4 纯函数抽取风险超预期，
  降级方案 = 仅在 rebuild_monitors 内修两处（删强清 + refresh 提前），放弃抽取，
  AC4 单测改为「构造 App 不可行 ⇒ 降级为脚本钉行为 + 注释登记」并在 design.md 记录。
